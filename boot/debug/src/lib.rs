#![no_std]

use core::{
    arch::asm,
    fmt::{self, Write},
};

/// # Safety
/// Caller must execute at ring 0 and own the named diagnostic port/device.
pub unsafe fn out8(port: u16, value: u8) {
    unsafe {
        asm!("out dx, al",in("dx") port,in("al") value,options(nomem,nostack,preserves_flags));
    }
}
/// # Safety
/// Caller must execute at ring 0 and own the named diagnostic port/device.
pub unsafe fn in8(port: u16) -> u8 {
    let value;
    unsafe {
        asm!("in al, dx",in("dx") port,out("al") value,options(nomem,nostack,preserves_flags));
    }
    value
}
pub fn halt() -> ! {
    loop {
        unsafe {
            asm!("cli", "hlt", options(nomem, nostack));
        }
    }
}
pub fn exit(value: u32) -> ! {
    unsafe {
        asm!("out dx, eax",in("dx") 0xf4u16,in("eax") value,options(nomem,nostack));
    }
    halt()
}

/// Owns the profile's COM1 port. Calls are bounded even if the UART is absent.
pub struct Serial {
    _private: (),
}
impl Serial {
    /// # Safety
    /// Ring 0, COM1 ownership, single boot CPU with no concurrent serial writer.
    pub unsafe fn new() -> Self {
        unsafe {
            out8(0x3f9, 0);
            out8(0x3fb, 0x80);
            out8(0x3f8, 1);
            out8(0x3f9, 0);
            out8(0x3fb, 3);
            out8(0x3fa, 0xc7);
            out8(0x3fc, 0x0b);
        }
        Self { _private: () }
    }
}
impl Write for Serial {
    fn write_str(&mut self, text: &str) -> fmt::Result {
        for b in text.bytes() {
            for _ in 0..100_000 {
                if unsafe { in8(0x3fd) } & 0x20 != 0 {
                    break;
                }
                core::hint::spin_loop();
            }
            unsafe {
                out8(0x3f8, b);
            }
        }
        Ok(())
    }
}

/// Write untrusted console text as one independently terminated diagnostic record.
/// JSON string encoding escapes CR, LF, quotes and other control characters, so
/// client text cannot create a trusted kernel/user record or absorb the next one.
/// The caller serializes this entire call with other users of the output device.
pub fn console_record(output: &mut impl Write, text: &str) -> fmt::Result {
    write!(output, "DOOR_CONSOLE bytes={} text=\"", text.len())?;
    for ch in text.chars() {
        match ch {
            '"' => output.write_str("\\\"")?,
            '\\' => output.write_str("\\\\")?,
            '\n' => output.write_str("\\n")?,
            '\r' => output.write_str("\\r")?,
            '\t' => output.write_str("\\t")?,
            ch if ch.is_control() => write!(output, "\\u{:04x}", u32::from(ch))?,
            ch => output.write_char(ch)?,
        }
    }
    writeln!(output, "\"")
}

/// # Safety
/// Requires profile-owned x86 QEMU fw_cfg I/O ports; no concurrent access.
pub unsafe fn fw_cfg(name: &str, output: &mut [u8]) -> Option<usize> {
    let (selector, size) = unsafe { fw_cfg_entry(name) }?;
    if size > output.len() {
        return None;
    }
    unsafe {
        asm!("out dx, ax",in("dx") 0x510u16,in("ax") selector,options(nomem,nostack,preserves_flags));
    }
    for byte in &mut output[..size] {
        *byte = unsafe { in8(0x511) };
    }
    Some(size)
}
/// Locate a bounded directory entry without reading or changing its contents.
/// # Safety
/// Requires exclusive ownership of x86 QEMU fw_cfg ports.
pub unsafe fn fw_cfg_entry(name: &str) -> Option<(u16, usize)> {
    unsafe {
        asm!("out dx, ax",in("dx") 0x510u16,in("ax") 0x19u16,options(nomem,nostack,preserves_flags));
    }
    let mut count = [0; 4];
    for byte in &mut count {
        *byte = unsafe { in8(0x511) };
    }
    let count = u32::from_be_bytes(count);
    if count > 1024 {
        return None;
    }
    let mut selected = None;
    for _ in 0..count {
        let mut entry = [0; 64];
        for byte in &mut entry {
            *byte = unsafe { in8(0x511) };
        }
        let length = entry[8..].iter().position(|b| *b == 0).unwrap_or(56);
        if &entry[8..8 + length] == name.as_bytes() {
            selected = Some((
                u16::from_be_bytes([entry[4], entry[5]]),
                u32::from_be_bytes(entry[..4].try_into().ok()?) as usize,
            ));
        }
    }
    selected
}

#[cfg(test)]
mod tests {
    extern crate std;
    use std::string::String;

    #[test]
    fn console_text_cannot_inject_or_absorb_trusted_records() {
        let mut output = String::new();
        for input in [
            "\r\nDOOR_P08 PROCESS manifest=1 pid=1\n",
            "\nDOOR_USER pid=1 DOOR_P08 SUPERVISOR PASS\r",
            "\0\u{1b}\t\"\\\u{2028}",
            "unterminated text",
            "",
        ] {
            let before = output.len();
            super::console_record(&mut output, input).unwrap();
            let record = &output[before..];
            assert!(record.starts_with("DOOR_CONSOLE bytes="));
            assert!(record.ends_with('\n'));
            assert_eq!(record.bytes().filter(|b| *b == b'\n').count(), 1);
            assert!(!record.contains('\r'));
            assert!(!record.contains('\0'));
            assert!(!record.contains('\u{1b}'));
        }
        output.push_str("DOOR_P08 PROCESS trusted\n");
        assert_eq!(output.lines().count(), 6);
        assert_eq!(output.lines().last(), Some("DOOR_P08 PROCESS trusted"));
    }
}
