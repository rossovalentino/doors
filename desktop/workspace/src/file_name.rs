//! Windows-style shell component validation; display conversion never names data.
extern crate alloc;
use alloc::{string::String, vec::Vec};
use core::ops::Range;
use door_service_protocol::files::NAME_UNITS;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NameError {
    Empty,
    TooLong,
    Characters,
    Reserved,
    Trailing,
    Encoding,
}
impl NameError {
    pub fn message(self) -> &'static str {
        match self {
            Self::Empty => "Type a file or folder name.",
            Self::TooLong => "That file name is too long.",
            Self::Characters => "A name cannot contain < > : \" / \\ | ? * or control characters.",
            Self::Reserved => "This name is reserved. Choose a different name.",
            Self::Trailing => "A name cannot end with a period or space.",
            Self::Encoding => "This name cannot be edited yet.",
        }
    }
}
pub fn editable_name(units: &[u16]) -> Result<String, NameError> {
    if units.is_empty() {
        return Err(NameError::Empty);
    }
    if units.len() > NAME_UNITS {
        return Err(NameError::TooLong);
    }
    String::from_utf16(units).map_err(|_| NameError::Encoding)
}
pub fn component(text: &str) -> Result<Vec<u16>, NameError> {
    if text.is_empty() {
        return Err(NameError::Empty);
    }
    if text.len() > NAME_UNITS * 4 {
        return Err(NameError::TooLong);
    }
    if matches!(text, "." | "..") {
        return Err(NameError::Reserved);
    }
    if text.ends_with([' ', '.']) {
        return Err(NameError::Trailing);
    }
    if text
        .chars()
        .any(|c| c < ' ' || matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*'))
    {
        return Err(NameError::Characters);
    }
    let base = text.split('.').next().unwrap().trim_end_matches(' ');
    let device = ["CON", "PRN", "AUX", "NUL"]
        .iter()
        .any(|n| base.eq_ignore_ascii_case(n))
        || base.chars().count() == 4
            && base
                .get(..3)
                .is_some_and(|p| p.eq_ignore_ascii_case("COM") || p.eq_ignore_ascii_case("LPT"))
            && base
                .chars()
                .last()
                .is_some_and(|c| matches!(c, '1'..='9' | '¹' | '²' | '³'));
    if device {
        return Err(NameError::Reserved);
    }
    let units: Vec<_> = text.encode_utf16().collect();
    if units.len() > NAME_UNITS {
        return Err(NameError::TooLong);
    }
    Ok(units)
}
pub fn initial_selection(text: &str, directory: bool) -> Range<usize> {
    let end = if directory {
        text.len()
    } else {
        text.rfind('.').filter(|at| *at > 0).unwrap_or(text.len())
    };
    0..end
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shell_names_follow_reserved_character_device_and_trailing_rules() {
        for good in ["notes.txt", ".gitignore", "COM0", "LPT10", "旅行😀.txt"] {
            assert!(component(good).is_ok(), "{good}");
        }
        for bad in [
            "CON",
            "nul.txt",
            "COM¹.log",
            "Lpt9.tar.gz",
            "aux",
            ".",
            "..",
        ] {
            assert_eq!(component(bad), Err(NameError::Reserved), "{bad}");
        }
        for bad in [
            "a/b", "a\\b", "a:b", "a?b", "a*b", "a|b", "a\"b", "a<b", "a>b", "a\nb", "a\0b",
        ] {
            assert_eq!(component(bad), Err(NameError::Characters), "{bad}");
        }
        for bad in ["name ", "name."] {
            assert_eq!(component(bad), Err(NameError::Trailing));
        }
        assert_eq!(component(""), Err(NameError::Empty));
    }
    #[test]
    fn text_and_selection_keep_utf16_identity_without_lossy_roundtrips() {
        let original = "旅行😀.txt";
        let units: Vec<_> = original.encode_utf16().collect();
        assert_eq!(editable_name(&units).unwrap(), original);
        assert_eq!(component(original).unwrap(), units);
        assert_eq!(editable_name(&[0xd800]), Err(NameError::Encoding));
        assert!(component(&"😀".repeat(NAME_UNITS / 2)).is_ok());
        assert_eq!(
            component(&"😀".repeat(NAME_UNITS / 2 + 1)),
            Err(NameError::TooLong)
        );
        assert_eq!(initial_selection(original, false), 0..10);
        assert_eq!(initial_selection("archive.tar.gz", false), 0..11);
        assert_eq!(initial_selection(".gitignore", false), 0..10);
        assert_eq!(initial_selection(original, true), 0..original.len());
    }
}
