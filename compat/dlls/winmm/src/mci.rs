//! MCI command strings (mciSendString) for the two devices Doors has, the
//! waveaudio and sequencer drivers: the parse of a command line into its
//! verb, device and arguments, and the MCIERR codes with Windows' texts.
use alloc::{string::String, vec::Vec};

/// MCIERR_* (MCIERR_BASE 256 + n).
pub mod err {
    pub const INVALID_DEVICE_ID: u32 = 257;
    pub const UNRECOGNIZED_KEYWORD: u32 = 259;
    pub const UNRECOGNIZED_COMMAND: u32 = 261;
    pub const HARDWARE: u32 = 262;
    pub const INVALID_DEVICE_NAME: u32 = 263;
    pub const OUT_OF_MEMORY: u32 = 264;
    pub const DEVICE_OPEN: u32 = 265;
    pub const CANNOT_LOAD_DRIVER: u32 = 266;
    pub const MISSING_COMMAND_STRING: u32 = 267;
    pub const PARAM_OVERFLOW: u32 = 268;
    pub const MISSING_STRING_ARGUMENT: u32 = 269;
    pub const BAD_INTEGER: u32 = 270;
    pub const PARSER_INTERNAL: u32 = 271;
    pub const DRIVER_INTERNAL: u32 = 272;
    pub const MISSING_PARAMETER: u32 = 273;
    pub const UNSUPPORTED_FUNCTION: u32 = 274;
    pub const FILE_NOT_FOUND: u32 = 275;
    pub const DEVICE_NOT_READY: u32 = 276;
    pub const INTERNAL: u32 = 277;
    pub const DRIVER: u32 = 278;
    pub const CANNOT_USE_ALL: u32 = 279;
    pub const MULTIPLE: u32 = 280;
    pub const EXTENSION_NOT_FOUND: u32 = 281;
    pub const OUTOFRANGE: u32 = 282;
    pub const FLAGS_NOT_COMPATIBLE: u32 = 284;
    pub const FILE_NOT_SAVED: u32 = 286;
    pub const DEVICE_TYPE_REQUIRED: u32 = 287;
    pub const DEVICE_LOCKED: u32 = 288;
    pub const DUPLICATE_ALIAS: u32 = 289;
    pub const BAD_CONSTANT: u32 = 290;
    pub const MUST_USE_SHAREABLE: u32 = 291;
    pub const MISSING_DEVICE_NAME: u32 = 292;
    pub const BAD_TIME_FORMAT: u32 = 293;
    pub const NO_CLOSING_QUOTE: u32 = 294;
    pub const DUPLICATE_FLAGS: u32 = 295;
    pub const INVALID_FILE: u32 = 296;
    pub const NULL_PARAMETER_BLOCK: u32 = 297;
    pub const UNNAMED_RESOURCE: u32 = 298;
    pub const NEW_REQUIRES_ALIAS: u32 = 299;
    pub const NOTIFY_ON_AUTO_OPEN: u32 = 300;
    pub const NO_ELEMENT_ALLOWED: u32 = 301;
    pub const NONAPPLICABLE_FUNCTION: u32 = 302;
    pub const ILLEGAL_FOR_AUTO_OPEN: u32 = 303;
    pub const FILENAME_REQUIRED: u32 = 304;
    pub const EXTRA_CHARACTERS: u32 = 305;
    pub const DEVICE_NOT_INSTALLED: u32 = 306;
    pub const GET_CD: u32 = 307;
    pub const SET_CD: u32 = 308;
    pub const SET_DRIVE: u32 = 309;
    pub const DEVICE_LENGTH: u32 = 310;
    pub const DEVICE_ORD_LENGTH: u32 = 311;
    pub const NO_INTEGER: u32 = 312;
    pub const WAVE_OUTPUTSINUSE: u32 = 320;
    pub const WAVE_SETOUTPUTINUSE: u32 = 321;
    pub const WAVE_INPUTSINUSE: u32 = 322;
    pub const WAVE_INPUTSUNSUITABLE: u32 = 325;
    pub const SEQ_PORT_INUSE: u32 = 336;
    pub const SEQ_TIMER: u32 = 339;
    pub const NO_WINDOW: u32 = 346;
}

/// The error texts mciGetErrorString answers (Windows' wording).
pub fn error_text(code: u32) -> Option<&'static str> {
    Some(match code {
        err::INVALID_DEVICE_ID => {
            "Invalid MCI device ID.  Use the ID returned when opening the MCI device."
        }
        err::UNRECOGNIZED_KEYWORD => "The driver cannot recognize the specified command parameter.",
        err::UNRECOGNIZED_COMMAND => "The driver cannot recognize the specified command.",
        err::HARDWARE => {
            "There is a problem with your media device.  Make sure it is working correctly or contact the device manufacturer."
        }
        err::INVALID_DEVICE_NAME => "The specified device is not open or is not recognized by MCI.",
        err::OUT_OF_MEMORY => {
            "Not enough memory is available for this task.  Quit one or more applications to increase available memory, and then try again."
        }
        err::DEVICE_OPEN => {
            "The device name is already being used as an alias by this application.  Use a unique alias."
        }
        err::CANNOT_LOAD_DRIVER => "The specified device driver will not load properly.",
        err::MISSING_COMMAND_STRING => "No command was specified.",
        err::PARAM_OVERFLOW => {
            "The output string was too large to fit in the return buffer.  Increase the size of the buffer."
        }
        err::MISSING_STRING_ARGUMENT => {
            "The specified command requires a character-string parameter.  Please provide one."
        }
        err::BAD_INTEGER => "The specified integer is invalid for this command.",
        err::PARSER_INTERNAL => {
            "The device driver returned an invalid return type.  Check with the device manufacturer about obtaining a new driver."
        }
        err::DRIVER_INTERNAL => {
            "There is a problem with the device driver.  Check with the device manufacturer about obtaining a new driver."
        }
        err::MISSING_PARAMETER => "The specified command requires a parameter.  Please supply one.",
        err::UNSUPPORTED_FUNCTION => {
            "The MCI device you are using does not support the specified command."
        }
        err::FILE_NOT_FOUND => {
            "Cannot find the specified file.  Make sure the path and filename are correct."
        }
        err::DEVICE_NOT_READY => "The device driver is not ready.",
        err::INTERNAL => "A problem occurred in initializing MCI.  Try restarting Doors.",
        err::DRIVER => {
            "There is a problem with the device driver.  The driver has closed.  Cannot access error."
        }
        err::CANNOT_USE_ALL => "Cannot use 'all' as the device name with the specified command.",
        err::MULTIPLE => {
            "Errors occurred in more than one device.  Specify each command and device separately to determine which devices caused the error."
        }
        err::EXTENSION_NOT_FOUND => {
            "Cannot determine the device type from the given filename extension."
        }
        err::OUTOFRANGE => "The specified parameter is out of range for the specified command.",
        err::FLAGS_NOT_COMPATIBLE => "The specified parameters cannot be used together.",
        err::FILE_NOT_SAVED => {
            "Cannot save the specified file.  Make sure you have enough disk space or are still connected to the network."
        }
        err::DEVICE_TYPE_REQUIRED => {
            "Cannot find the specified device.  Make sure it is installed or that the device name is spelled correctly."
        }
        err::DEVICE_LOCKED => {
            "The specified device is now being closed.  Wait a few seconds, and then try again."
        }
        err::DUPLICATE_ALIAS => {
            "The specified alias is already being used in this application.  Use a unique alias."
        }
        err::BAD_CONSTANT => "The specified parameter is invalid for this command.",
        err::MUST_USE_SHAREABLE => {
            "The device driver is already in use.  To share it, use the 'shareable' parameter with each 'open' command."
        }
        err::MISSING_DEVICE_NAME => {
            "The specified command requires an alias, file, driver, or device name.  Please supply one."
        }
        err::BAD_TIME_FORMAT => {
            "The specified value for the time format is invalid.  Refer to the MCI documentation for valid formats."
        }
        err::NO_CLOSING_QUOTE => {
            "A closing double-quotation mark is missing from the parameter value.  Please supply one."
        }
        err::DUPLICATE_FLAGS => "A parameter or value was specified twice.  Only specify it once.",
        err::INVALID_FILE => {
            "The specified file cannot be played on the specified MCI device.  The file may be corrupt, not in the correct format, or no file handler available for this format."
        }
        err::NULL_PARAMETER_BLOCK => "A null parameter block was passed to MCI.",
        err::UNNAMED_RESOURCE => "Cannot save an unnamed file.  Supply a filename.",
        err::NEW_REQUIRES_ALIAS => "You must specify an alias when using the 'new' parameter.",
        err::NOTIFY_ON_AUTO_OPEN => "Cannot use the 'notify' flag with auto-opened devices.",
        err::NO_ELEMENT_ALLOWED => "Cannot use a filename with the specified device.",
        err::NONAPPLICABLE_FUNCTION => {
            "Cannot carry out the commands in the order specified.  Correct the command sequence, and then try again."
        }
        err::ILLEGAL_FOR_AUTO_OPEN => {
            "Cannot carry out the specified command on an auto-opened device.  Wait until the device is closed, and then try again."
        }
        err::FILENAME_REQUIRED => {
            "The filename is invalid.  Make sure the filename is not longer than 8 characters, followed by a period and an extension."
        }
        err::EXTRA_CHARACTERS => {
            "Cannot specify extra characters after a string enclosed in quotation marks."
        }
        err::DEVICE_NOT_INSTALLED => {
            "The specified device is not installed on the system.  Use the Drivers option in Control Panel to install the device."
        }
        err::GET_CD => {
            "Cannot access the specified file or MCI device.  Try changing directories or restarting your computer."
        }
        err::SET_CD => {
            "Cannot access the specified file or MCI device because the application cannot change directories."
        }
        err::SET_DRIVE => {
            "Cannot access specified file or MCI device because the application cannot change drives."
        }
        err::DEVICE_LENGTH => "Specify a device or driver name that is less than 79 characters.",
        err::DEVICE_ORD_LENGTH => {
            "Specify a device or driver name that is less than 69 characters."
        }
        err::NO_INTEGER => {
            "The specified command requires an integer parameter.  Please provide one."
        }
        err::WAVE_OUTPUTSINUSE => {
            "All wave devices that can play files in the current format are in use.  Wait until a wave device is free, and then try again."
        }
        err::WAVE_SETOUTPUTINUSE => {
            "Cannot set the current wave device for play back because it is in use.  Wait until the device is free, and then try again."
        }
        err::SEQ_PORT_INUSE => {
            "The specified MIDI port is already in use.  Wait until it is free; then try again."
        }
        err::SEQ_TIMER => {
            "All multimedia timers are being used by other applications.  Quit one of these applications; then, try again."
        }
        err::NO_WINDOW => "There is no display window.",
        _ => return None,
    })
}

/// A command line split into words: double quotes group (and are
/// removed), whitespace separates.
pub fn words(line: &str) -> Result<Vec<String>, u32> {
    let mut out = Vec::new();
    let mut chars = line.chars().peekable();
    loop {
        while chars.peek().is_some_and(|c| c.is_whitespace()) {
            chars.next();
        }
        let Some(&first) = chars.peek() else {
            return Ok(out);
        };
        let mut word = String::new();
        if first == '"' {
            chars.next();
            loop {
                match chars.next() {
                    Some('"') => break,
                    Some(c) => word.push(c),
                    None => return Err(err::NO_CLOSING_QUOTE),
                }
            }
            if chars.peek().is_some_and(|c| !c.is_whitespace()) {
                return Err(err::EXTRA_CHARACTERS);
            }
        } else {
            while let Some(&c) = chars.peek() {
                if c.is_whitespace() {
                    break;
                }
                word.push(c);
                chars.next();
            }
        }
        out.push(word);
    }
}

/// The device type a file extension selects (the [mci extensions] of
/// win.ini / the registry's MCI Extensions).
pub fn type_for_extension(path: &str) -> Option<&'static str> {
    let extension = path.rsplit_once('.')?.1;
    if extension.eq_ignore_ascii_case("wav") {
        Some("waveaudio")
    } else if ["mid", "midi", "rmi"]
        .iter()
        .any(|e| extension.eq_ignore_ascii_case(e))
    {
        Some("sequencer")
    } else {
        None
    }
}

/// The MCI devices Doors installs, in `sysinfo all name` order.
pub const DEVICES: [&str; 2] = ["Sequencer", "WaveAudio"];

/// Time formats.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TimeFormat {
    Milliseconds,
    Samples,
    Bytes,
}
impl TimeFormat {
    pub fn parse(word: &str) -> Option<Self> {
        Some(match word.to_ascii_lowercase().as_str() {
            "milliseconds" | "ms" => Self::Milliseconds,
            "samples" => Self::Samples,
            "bytes" => Self::Bytes,
            _ => return None,
        })
    }
    pub fn name(self) -> &'static str {
        match self {
            Self::Milliseconds => "milliseconds",
            Self::Samples => "samples",
            Self::Bytes => "bytes",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn command_words_and_quotes() {
        assert_eq!(
            words("open  tone.wav type waveaudio alias t").unwrap(),
            ["open", "tone.wav", "type", "waveaudio", "alias", "t"]
        );
        assert_eq!(
            words(r#"open "C:\My Music\a.mid" alias s"#).unwrap(),
            ["open", r"C:\My Music\a.mid", "alias", "s"]
        );
        assert_eq!(words(r#"open "x.wav"y"#), Err(err::EXTRA_CHARACTERS));
        assert_eq!(words(r#"open "x.wav"#), Err(err::NO_CLOSING_QUOTE));
        assert!(words("   ").unwrap().is_empty());
    }
    #[test]
    fn extensions_formats_and_texts() {
        assert_eq!(type_for_extension(r"C:\a\b.WAV"), Some("waveaudio"));
        assert_eq!(type_for_extension("song.mid"), Some("sequencer"));
        assert_eq!(type_for_extension("song.rmi"), Some("sequencer"));
        assert_eq!(type_for_extension("movie.avi"), None);
        assert_eq!(TimeFormat::parse("MS"), Some(TimeFormat::Milliseconds));
        assert_eq!(TimeFormat::parse("frames"), None);
        assert_eq!(
            error_text(err::INVALID_DEVICE_NAME),
            Some("The specified device is not open or is not recognized by MCI.")
        );
        assert_eq!(error_text(256), None);
    }
}
