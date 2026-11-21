#![no_std]
//! Original oleacc.dll (P18): Microsoft Active Accessibility. Doors has no
//! MSAA clients or standard proxies yet: objects for windows are not
//! created (E_NOTIMPL / E_FAIL) and LresultFromObject returns E_NOTIMPL,
//! as a window without an accessibility server would.
//!
//! The same sources build the x64 and the x86 DLL.

#[cfg(all(feature = "guest-pe", target_os = "windows", not(test)))]
mod guest;
