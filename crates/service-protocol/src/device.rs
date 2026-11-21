//! The device name (Settings > System > About > Rename this PC; File
//! Explorer's This PC details). One machine-wide name, stored at
//! `HKLM\Software\Doors\ComputerName`, REG_SZ value `ComputerName`; absent
//! means [`DEFAULT_NAME`]. HKLM is writable only by system subjects, so the
//! Settings app asks Init ([`SET_NAME`]), which validates the name again
//! and writes it. Validation follows Windows: 1..=15 characters, letters
//! A-Z/a-z, digits and hyphens only, not digits only.
use core::fmt;

/// Init operation (Settings app of the live session only): body = the new
/// name as UTF-8 (1..=`MAX_LENGTH` bytes). Replies empty; `InvalidRequest`
/// for a name [`Name::parse`] rejects, `Denied` for any other caller.
pub const SET_NAME: u32 = 0x111A;
/// The name before the first rename.
pub const DEFAULT_NAME: &str = "DOORS-PC";
/// NetBIOS limit Windows enforces in Rename this PC.
pub const MAX_LENGTH: usize = 15;
/// HKLM key and REG_SZ value holding the name.
pub const KEY_PATH: &str = "Software\\Doors\\ComputerName";
pub const VALUE: &str = "ComputerName";
/// REG_SZ bytes of the longest name: UTF-16LE plus the terminating NUL.
pub const REGISTRY_BYTES: usize = (MAX_LENGTH + 1) * 2;

/// Why Rename this PC refuses a name; [`NameError::message`] is the text
/// shown under the box.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NameError {
    Empty,
    InvalidCharacters,
    TooLong,
    OnlyDigits,
}
impl NameError {
    pub const fn message(self) -> &'static str {
        match self {
            Self::Empty => "Enter a name for your PC.",
            Self::InvalidCharacters => {
                "Your PC name can't contain spaces or any of the following characters: \
                 ` ~ ! @ # $ % ^ & * ( ) = + _ [ ] { } \\ | ; : . ' \" , < > / ?"
            }
            Self::TooLong => "Your PC name can't be longer than 15 characters.",
            Self::OnlyDigits => "Your PC name can't be only numbers.",
        }
    }
}

/// A valid device name (ASCII, at most `MAX_LENGTH` bytes), kept as typed.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Name {
    bytes: [u8; MAX_LENGTH],
    len: u8,
}
impl Name {
    /// Validate `text` as Windows does (the checks run in this order: empty,
    /// characters, length, digits only).
    pub fn parse(text: &str) -> Result<Self, NameError> {
        if text.is_empty() {
            return Err(NameError::Empty);
        }
        if !text.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-') {
            return Err(NameError::InvalidCharacters);
        }
        if text.len() > MAX_LENGTH {
            return Err(NameError::TooLong);
        }
        if text.bytes().all(|b| b.is_ascii_digit()) {
            return Err(NameError::OnlyDigits);
        }
        let mut name = Self {
            bytes: [0; MAX_LENGTH],
            len: text.len() as u8,
        };
        name.bytes[..text.len()].copy_from_slice(text.as_bytes());
        Ok(name)
    }
    pub fn as_str(&self) -> &str {
        // Only validated ASCII is ever stored.
        core::str::from_utf8(&self.bytes[..usize::from(self.len)]).unwrap_or(DEFAULT_NAME)
    }
    /// REG_SZ data (UTF-16LE with the terminating NUL); returns its length.
    pub fn registry_data(&self, out: &mut [u8; REGISTRY_BYTES]) -> usize {
        out.fill(0);
        for (i, b) in self.as_str().bytes().enumerate() {
            out[i * 2] = b;
        }
        (usize::from(self.len) + 1) * 2
    }
    /// Decode stored REG_SZ data; anything that is not a valid name (a
    /// hand-edited value) reads as `None`, so readers fall back to the
    /// default rather than show it.
    pub fn from_registry(data: &[u8]) -> Option<Self> {
        if !data.len().is_multiple_of(2) {
            return None;
        }
        let mut text = [0u8; MAX_LENGTH];
        let mut len = 0;
        for unit in data.chunks_exact(2) {
            let unit = u16::from_le_bytes([unit[0], unit[1]]);
            if unit == 0 {
                break;
            }
            let byte = u8::try_from(unit).ok().filter(u8::is_ascii)?;
            *text.get_mut(len)? = byte;
            len += 1;
        }
        Self::parse(core::str::from_utf8(&text[..len]).ok()?).ok()
    }
}
impl Default for Name {
    fn default() -> Self {
        let mut name = Self {
            bytes: [0; MAX_LENGTH],
            len: DEFAULT_NAME.len() as u8,
        };
        name.bytes[..DEFAULT_NAME.len()].copy_from_slice(DEFAULT_NAME.as_bytes());
        name
    }
}
impl fmt::Debug for Name {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self.as_str(), f)
    }
}
impl fmt::Display for Name {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}
/// Rename this PC's check: `Ok` or the error to show.
pub fn validate(text: &str) -> Result<(), NameError> {
    Name::parse(text).map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn names_validate_like_windows_rename_this_pc() {
        for good in [
            "DOORS-PC",
            "a",
            "Bob-Laptop",
            "PC2",
            "123456789012345A",
            "-x-",
        ]
        .into_iter()
        .filter(|n| n.len() <= MAX_LENGTH)
        {
            assert_eq!(
                Name::parse(good).map(|n| n.as_str() == good),
                Ok(true),
                "{good}"
            );
        }
        assert_eq!(validate(""), Err(NameError::Empty));
        for bad in ["my pc", "pc_1", "pc.local", "caf\u{e9}", "a/b", "tab\t"] {
            assert_eq!(validate(bad), Err(NameError::InvalidCharacters), "{bad}");
        }
        assert_eq!(validate("ABCDEFGHIJKLMNOP"), Err(NameError::TooLong));
        assert_eq!(validate("ABCDEFGHIJKLMNO"), Ok(()));
        assert_eq!(validate("2024"), Err(NameError::OnlyDigits));
        // Characters are reported before the length, as the box shows them.
        assert_eq!(
            validate("a very long name with spaces"),
            Err(NameError::InvalidCharacters)
        );
        for e in [
            NameError::Empty,
            NameError::InvalidCharacters,
            NameError::TooLong,
            NameError::OnlyDigits,
        ] {
            assert!(e.message().starts_with("Your PC name") || e == NameError::Empty);
        }
        assert!(NameError::InvalidCharacters.message().contains("spaces"));
        assert_eq!(Name::default().as_str(), DEFAULT_NAME);
        assert_eq!(validate(DEFAULT_NAME), Ok(()));
    }
    #[test]
    fn registry_data_round_trips_and_rejects_hand_edited_values() {
        let name = Name::parse("Bob-Laptop").unwrap();
        let mut out = [0; REGISTRY_BYTES];
        let n = name.registry_data(&mut out);
        assert_eq!(n, 22);
        assert_eq!(&out[..4], &[b'B', 0, b'o', 0]);
        assert_eq!(&out[20..22], &[0, 0]);
        assert_eq!(Name::from_registry(&out[..n]), Some(name));
        let longest = Name::parse("ABCDEFGHIJKLMNO").unwrap();
        assert_eq!(longest.registry_data(&mut out), REGISTRY_BYTES);
        assert_eq!(Name::from_registry(&out), Some(longest));
        // Not NUL-terminated is accepted (the value ends at its length).
        assert_eq!(
            Name::from_registry(&[b'P', 0, b'C', 0]).unwrap().as_str(),
            "PC"
        );
        for bad in [
            &[b'a', 0, b' ', 0, 0, 0][..],
            &[0x41, 0x30, 0, 0],
            &[b'1', 0, 0, 0],
            &[0, 0],
            b"a",
            &[],
        ] {
            assert_eq!(Name::from_registry(bad), None, "{bad:?}");
        }
        let mut long = [0u8; 40];
        for i in 0..20 {
            long[i * 2] = b'a';
        }
        assert_eq!(Name::from_registry(&long), None);
    }
}
