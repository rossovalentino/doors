#![no_std]
//! Original wldp.dll: Windows Lockdown Policy as a system without Windows
//! Defender Application Control or AppLocker policy answers it. PowerShell
//! asks at start (WldpGetLockdownPolicy) whether to run in Constrained
//! Language mode, and before running a script whether the file may run;
//! Doors enforces no code-integrity policy, so the state is "no lockdown",
//! files may execute and COM classes are approved.
//!
//! The same sources build the x64 and the x86 DLL.

#[cfg(all(feature = "guest-pe", target_os = "windows", not(test)))]
mod guest;

pub const S_OK: u32 = 0;
pub const E_INVALIDARG: u32 = 0x8007_0057;
/// HRESULT_FROM_WIN32(ERROR_NOT_FOUND): no such application setting.
pub const E_NOT_FOUND: u32 = 0x8007_0490;
/// WLDP_LOCKDOWN_UNDEFINED: no policy.
pub const NO_LOCKDOWN: u32 = 0;
/// WLDP_EXECUTION_POLICY_ALLOWED.
pub const EXECUTION_ALLOWED: u32 = 1;
