#![no_std]
//! Original mfplat.dll (P18): the Media Foundation platform is not
//! available on Doors yet. MFStartup reports E_NOTIMPL, so programs use
//! their own decoders (Chromium uses FFmpeg); the object factories report
//! E_NOTIMPL too. MFGetSystemTime and MFFrameRateToAverageTimePerFrame
//! compute their answers.
//!
//! The same sources build the x64 and the x86 DLL.

#[cfg(all(feature = "guest-pe", target_os = "windows", not(test)))]
mod guest;

/// MFFrameRateToAverageTimePerFrame: 100 ns units per frame, rounded as
/// Windows does (the common NTSC rates map to their exact values).
pub fn average_time_per_frame(numerator: u32, denominator: u32) -> Option<u64> {
    if numerator == 0 {
        return None;
    }
    Some(match (numerator, denominator) {
        (60000, 1001) => 166_833,
        (30000, 1001) => 333_667,
        (24000, 1001) => 417_188,
        _ => {
            (u64::from(denominator) * 10_000_000 + u64::from(numerator) / 2) / u64::from(numerator)
        }
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn frame_times() {
        assert_eq!(super::average_time_per_frame(30, 1), Some(333_333));
        assert_eq!(super::average_time_per_frame(30000, 1001), Some(333_667));
        assert_eq!(super::average_time_per_frame(0, 1), None);
    }
}
