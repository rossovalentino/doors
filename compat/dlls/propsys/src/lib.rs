#![no_std]
//! Original propsys.dll (P18): PROPVARIANT helpers and the in-memory
//! property store (PSCreateMemoryPropertyStore), with VariantCompare over
//! oleaut32's VarCmp.
//!
//! The same sources build the x64 and the x86 DLL.
extern crate alloc;

#[cfg(all(feature = "guest-pe", target_os = "windows", not(test)))]
mod guest;
