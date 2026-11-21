#![no_std]
//! Doors' Universal C Runtime (`ucrtbase.dll`, the `api-ms-win-crt-*`
//! contracts): exported PE entry points exist only in guest builds; the
//! logic is `door_crt`'s, host-tested.
#[cfg(all(feature = "guest-pe", target_os = "windows", not(test)))]
mod guest;
