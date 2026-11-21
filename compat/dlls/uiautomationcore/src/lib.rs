#![no_std]
//! Original UIAutomationCore.dll (P18): the provider side of UI
//! Automation. No UI Automation client runs on Doors yet: events are
//! accepted with no listeners, UiaReturnRawElementProvider answers 0, and
//! the reserved values are static objects.
//!
//! The same sources build the x64 and the x86 DLL.

#[cfg(all(feature = "guest-pe", target_os = "windows", not(test)))]
mod guest;
