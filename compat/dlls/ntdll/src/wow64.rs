//! WOW64's file system redirector in 32-bit ntdll (P17): a 32-bit program's
//! C:\Windows\System32 is C:\Windows\SysWOW64 unless its thread turned
//! redirection off (Wow64DisableWow64FsRedirection), the exempt folders
//! stay where they are, and C:\Windows\Sysnative names the real System32
//! (`door_windows_abi::wow64::file_path`). Every path ntdll resolves passes
//! here; the x64 build has no redirector.
use super::files::PATH_TEXT;

/// Redirects the normalized Windows path `out[..length]` in place; its new
/// length (206, ERROR_FILENAME_EXCED_RANGE, when it no longer fits).
pub(super) fn redirect(out: &mut [u8; PATH_TEXT], length: usize) -> Result<usize, u32> {
    let mut redirected = [0u8; PATH_TEXT];
    let Ok(path) = core::str::from_utf8(&out[..length]) else {
        return Ok(length);
    };
    let on = super::arch::fs_redirection() == 0;
    match door_windows_abi::wow64::file_path(path, on, &mut redirected) {
        Ok(None) => Ok(length),
        Ok(Some(n)) => {
            out[..n].copy_from_slice(&redirected[..n]);
            Ok(n)
        }
        Err(_) => Err(206),
    }
}

/// Whether a UTF-16 path mentions System32 or Sysnative (ASCII case
/// insensitive): only those can redirect.
pub(super) fn names_system(path: &[u16]) -> bool {
    let lower = |u: u16| if (65..=90).contains(&u) { u + 32 } else { u };
    [&b"system32"[..], &b"sysnative"[..]].iter().any(|word| {
        path.windows(word.len()).any(|w| {
            w.iter()
                .zip(word.iter())
                .all(|(&u, &b)| lower(u) == u16::from(b))
        })
    })
}

/// RtlWow64EnableFsRedirectionEx: `disable` (0 turns redirection back on,
/// anything else off) becomes the thread's state, whose previous value goes
/// to `old` (a PVOID). STATUS_ACCESS_VIOLATION for a bad `old`.
#[unsafe(no_mangle)]
pub extern "system" fn RtlWow64EnableFsRedirectionEx(disable: usize, old: usize) -> u32 {
    let disable = door_windows_abi::widen(disable);
    let old = door_windows_abi::widen(old);
    {
        let previous = super::arch::fs_redirection();
        if old != 0
            && super::guest::copy(
                old,
                &(previous as usize) as *const usize as u64,
                core::mem::size_of::<usize>(),
            )
            .is_err()
        {
            return door_windows_abi::status::ACCESS_VIOLATION;
        }
        super::arch::set_fs_redirection(u32::from(disable != 0));
        door_windows_abi::status::SUCCESS
    }
}
