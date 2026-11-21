#![no_std]
//! Original dwmapi.dll. Doors' window server composes every window, so
//! programs see the Desktop Window Manager with composition always on:
//! frame extension, blur and window attributes are accepted, a window's
//! extended frame bounds are its window rectangle, nothing is cloaked,
//! composition timing reports the display's 60 Hz cadence, and DwmFlush
//! waits for the next frame.
//!
//! The same sources build the x64 and the x86 DLL.

#[cfg(all(feature = "guest-pe", target_os = "windows", not(test)))]
mod guest;

pub const S_OK: u32 = 0;
pub const E_INVALIDARG: u32 = 0x8007_0057;
pub const E_HANDLE: u32 = 0x8007_0006;
/// The composition rate (frames per second).
pub const REFRESH_HZ: u64 = 60;
/// The accent colour Windows 11 ships with (DwmGetColorizationColor's
/// 0xAARRGGBB): #0078D4 at about 77% opacity.
pub const COLORIZATION: u32 = 0xC400_78D4;

/// The fields of DWM_TIMING_INFO a caller reads, at their offsets (the
/// structure is packed): rateRefresh, qpcRefreshPeriod, rateCompose,
/// qpcVBlank, cRefresh, then qpcCompose and cFrame.
pub fn timing(frequency: u64, now: u64, out: &mut [u8]) -> bool {
    if out.len() < 64 || frequency == 0 {
        return false;
    }
    let period = frequency / REFRESH_HZ;
    let refreshes = now / period.max(1);
    let vblank = refreshes * period;
    let put32 =
        |out: &mut [u8], at: usize, v: u32| out[at..at + 4].copy_from_slice(&v.to_le_bytes());
    let put64 =
        |out: &mut [u8], at: usize, v: u64| out[at..at + 8].copy_from_slice(&v.to_le_bytes());
    put32(out, 4, REFRESH_HZ as u32);
    put32(out, 8, 1);
    put64(out, 12, period);
    put32(out, 20, REFRESH_HZ as u32);
    put32(out, 24, 1);
    put64(out, 28, vblank);
    put64(out, 36, refreshes);
    put64(out, 48, vblank);
    put64(out, 56, refreshes);
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn timing_reports_sixty_hertz_vblanks() {
        let mut out = [0u8; 292];
        assert!(timing(10_000_000, 1_234_567, &mut out));
        let u32_at = |at: usize| u32::from_le_bytes(out[at..at + 4].try_into().unwrap());
        let u64_at = |at: usize| u64::from_le_bytes(out[at..at + 8].try_into().unwrap());
        assert_eq!((u32_at(4), u32_at(8)), (60, 1));
        assert_eq!(u64_at(12), 166_666);
        let vblank = u64_at(28);
        assert!(vblank <= 1_234_567 && 1_234_567 - vblank < 166_666);
        assert!(!timing(10_000_000, 1, &mut [0u8; 16]));
    }
}
