//! FormatMessageW/A (P18 M0c): system message text for the common Win32
//! errors and HRESULTs (Windows' English wording), messages from a string,
//! inserts (`%1`..`%99` with `!printf!` formats, from a va_list or an
//! argument array), escapes, the width-mask rule and allocated buffers.
//! Message tables in other modules (FORMAT_MESSAGE_FROM_HMODULE) are not
//! read yet: such calls fail with ERROR_RESOURCE_TYPE_NOT_FOUND unless
//! FROM_SYSTEM is also given.

#[link(name = "ntdll", kind = "dylib")]
unsafe extern "system" {
    fn RtlSetLastWin32Error(value: u32);
}
fn set_error(value: u32) {
    unsafe { RtlSetLastWin32Error(value) };
}

const ALLOCATE_BUFFER: u32 = 0x100;
const IGNORE_INSERTS: u32 = 0x200;
const FROM_STRING: u32 = 0x400;
const FROM_HMODULE: u32 = 0x800;
const FROM_SYSTEM: u32 = 0x1000;
const ARGUMENT_ARRAY: u32 = 0x2000;
const MAX_WIDTH_MASK: u32 = 0xff;

/// Windows' English system messages for the codes runtimes and installers
/// format most (each ends with CR LF, as Windows' do).
const SYSTEM: &[(u32, &str)] = &[
    (0, "The operation completed successfully.\r\n"),
    (1, "Incorrect function.\r\n"),
    (2, "The system cannot find the file specified.\r\n"),
    (3, "The system cannot find the path specified.\r\n"),
    (4, "The system cannot open the file.\r\n"),
    (5, "Access is denied.\r\n"),
    (6, "The handle is invalid.\r\n"),
    (
        8,
        "Not enough memory resources are available to process this command.\r\n",
    ),
    (13, "The data is invalid.\r\n"),
    (
        14,
        "Not enough memory resources are available to complete this operation.\r\n",
    ),
    (15, "The system cannot find the drive specified.\r\n"),
    (18, "There are no more files.\r\n"),
    (19, "The media is write protected.\r\n"),
    (21, "The device is not ready.\r\n"),
    (
        32,
        "The process cannot access the file because it is being used by another process.\r\n",
    ),
    (
        33,
        "The process cannot access the file because another process has locked a portion of the file.\r\n",
    ),
    (38, "Reached the end of the file.\r\n"),
    (39, "The disk is full.\r\n"),
    (50, "The request is not supported.\r\n"),
    (53, "The network path was not found.\r\n"),
    (80, "The file exists.\r\n"),
    (87, "The parameter is incorrect.\r\n"),
    (109, "The pipe has been ended.\r\n"),
    (111, "The file name is too long.\r\n"),
    (112, "There is not enough space on the disk.\r\n"),
    (120, "This function is not supported on this system.\r\n"),
    (
        122,
        "The data area passed to a system call is too small.\r\n",
    ),
    (
        123,
        "The filename, directory name, or volume label syntax is incorrect.\r\n",
    ),
    (126, "The specified module could not be found.\r\n"),
    (127, "The specified procedure could not be found.\r\n"),
    (145, "The directory is not empty.\r\n"),
    (161, "The specified path is invalid.\r\n"),
    (
        183,
        "Cannot create a file when that file already exists.\r\n",
    ),
    (193, "%1 is not a valid Win32 application.\r\n"),
    (
        203,
        "The system could not find the environment option that was entered.\r\n",
    ),
    (206, "The filename or extension is too long.\r\n"),
    (231, "All pipe instances are busy.\r\n"),
    (232, "The pipe is being closed.\r\n"),
    (233, "No process is on the other end of the pipe.\r\n"),
    (234, "More data is available.\r\n"),
    (258, "The wait operation timed out.\r\n"),
    (259, "No more data is available.\r\n"),
    (267, "The directory name is invalid.\r\n"),
    (298, "Too many posts were made to a semaphore.\r\n"),
    (
        317,
        "The system cannot find message text for message number 0x%1 in the message file for %2.\r\n",
    ),
    (487, "Attempt to access invalid address.\r\n"),
    (535, "There is a process on other end of the pipe.\r\n"),
    (
        536,
        "Waiting for a process to open the other end of the pipe.\r\n",
    ),
    (
        995,
        "The I/O operation has been aborted because of either a thread exit or an application request.\r\n",
    ),
    (996, "Overlapped I/O event is not in a signaled state.\r\n"),
    (997, "Overlapped I/O operation is in progress.\r\n"),
    (998, "Invalid access to memory location.\r\n"),
    (1004, "Invalid flags.\r\n"),
    (
        1150,
        "The specified program requires a newer version of Windows.\r\n",
    ),
    (
        1155,
        "No application is associated with the specified file for this operation.\r\n",
    ),
    (1168, "Element not found.\r\n"),
    (1223, "The operation was canceled by the user.\r\n"),
    (1314, "A required privilege is not held by the client.\r\n"),
    (
        1460,
        "This operation returned because the timeout period expired.\r\n",
    ),
    (2147500033, "Not implemented\r\n"),
    (2147500034, "No such interface supported\r\n"),
    (2147500035, "Invalid pointer\r\n"),
    (2147500036, "Operation aborted\r\n"),
    (2147500037, "Unspecified error\r\n"),
    (2147549183, "Catastrophic failure\r\n"),
];
fn system_text(id: u32) -> Option<&'static str> {
    let id = if id & 0xffff_0000 == 0x8007_0000 {
        id & 0xffff
    } else {
        id
    };
    SYSTEM
        .iter()
        .find(|(code, _)| *code == id)
        .map(|(_, text)| *text)
}

/// The output being built, in UTF-16 units.
struct Out {
    units: [u16; 8192],
    len: usize,
}
impl Out {
    fn push(&mut self, unit: u16) -> Option<()> {
        *self.units.get_mut(self.len)? = unit;
        self.len += 1;
        Some(())
    }
    fn push_str(&mut self, text: &str) -> Option<()> {
        for unit in text.encode_utf16() {
            self.push(unit)?;
        }
        Some(())
    }
}
/// Where insert arguments come from.
struct Arguments {
    pointer: usize,
    array: bool,
}
impl Arguments {
    /// Argument `index` (1-based) as a pointer-sized value.
    fn get(&self, index: usize) -> usize {
        if self.pointer == 0 {
            return 0;
        }
        let base = if self.array {
            self.pointer
        } else {
            // A va_list*: the va_list is a pointer to pointer-sized slots.
            unsafe { (self.pointer as *const usize).read_unaligned() }
        };
        unsafe { ((base as *const usize).add(index - 1)).read_unaligned() }
    }
}
fn number(out: &mut Out, value: u64, radix: u32, upper: bool, signed: bool) -> Option<()> {
    let mut digits = [0u8; 24];
    let mut n = 0;
    let (negative, mut magnitude) = if signed && (value as i64) < 0 {
        (true, (value as i64).unsigned_abs())
    } else {
        (false, value)
    };
    loop {
        let d = (magnitude % u64::from(radix)) as u8;
        digits[n] = match d {
            0..=9 => b'0' + d,
            _ if upper => b'A' + d - 10,
            _ => b'a' + d - 10,
        };
        n += 1;
        magnitude /= u64::from(radix);
        if magnitude == 0 {
            break;
        }
    }
    if negative {
        out.push(u16::from(b'-'))?;
    }
    for d in digits[..n].iter().rev() {
        out.push(u16::from(*d))?;
    }
    Some(())
}
/// One insert with printf-style `format` (without the `!`s).
fn insert(out: &mut Out, value: usize, format: &[u16], wide_default: bool) -> Option<()> {
    let format: &[u16] = if format.is_empty() {
        &[b's' as u16]
    } else {
        format
    };
    let last = *format.last()? as u8;
    let long64 = format
        .windows(3)
        .any(|w| w == [b'I' as u16, b'6' as u16, b'4' as u16]);
    let value64 = if long64 || core::mem::size_of::<usize>() == 8 && format.contains(&(b'p' as u16))
    {
        value as u64
    } else if matches!(last, b'd' | b'i') {
        value as u32 as i32 as i64 as u64
    } else {
        u64::from(value as u32)
    };
    match last {
        b's' | b'S' => {
            let narrow = format.contains(&(b'h' as u16)) || (last == b'S') == wide_default;
            let wide = !narrow;
            if value == 0 {
                return out.push_str("(null)");
            }
            let mut i = 0;
            loop {
                let unit = if wide {
                    unsafe { ((value as *const u16).add(i)).read_unaligned() }
                } else {
                    u16::from(unsafe { ((value as *const u8).add(i)).read() })
                };
                if unit == 0 || i > 32_767 {
                    break;
                }
                out.push(unit)?;
                i += 1;
            }
            Some(())
        }
        b'c' | b'C' => out.push(value as u16),
        b'd' | b'i' => number(out, value64, 10, false, true),
        b'u' => number(out, value64, 10, false, false),
        b'x' => number(out, value64, 16, false, false),
        b'X' => number(out, value64, 16, true, false),
        b'o' => number(out, value64, 8, false, false),
        b'p' => {
            let width = core::mem::size_of::<usize>() * 2;
            let mut digits = [0u16; 16];
            let mut v = value as u64;
            for d in digits[..width].iter_mut().rev() {
                let n = (v & 15) as u8;
                *d = u16::from(if n < 10 { b'0' + n } else { b'A' + n - 10 });
                v >>= 4;
            }
            for d in &digits[..width] {
                out.push(*d)?;
            }
            Some(())
        }
        _ => None,
    }
}
/// Expand `source` into `out` under `flags` (the shared part of the W and
/// A forms): None when an insert or format cannot be expanded.
fn expand(
    source: &[u16],
    flags: u32,
    arguments: &Arguments,
    wide_default: bool,
    out: &mut Out,
) -> Option<()> {
    let width = flags & MAX_WIDTH_MASK;
    let mut i = 0;
    while i < source.len() {
        let unit = source[i];
        if unit == 0 {
            break;
        }
        if width == MAX_WIDTH_MASK && (unit == 13 || unit == 10) {
            // Line breaks in the text are ignored at this width.
            i += 1;
            continue;
        }
        if unit != b'%' as u16 {
            out.push(unit)?;
            i += 1;
            continue;
        }
        let Some(&next) = source.get(i + 1) else {
            out.push(unit)?;
            break;
        };
        i += 2;
        match next as u8 {
            b'0' => return Some(()),
            b'n' => {
                out.push(13)?;
                out.push(10)?;
            }
            b'r' => out.push(13)?,
            b't' => out.push(9)?,
            b'1'..=b'9' if flags & IGNORE_INSERTS == 0 => {
                let mut index = usize::from(next as u8 - b'0');
                if let Some(&d) = source.get(i)
                    && (b'0' as u16..=b'9' as u16).contains(&d)
                {
                    index = index * 10 + usize::from(d as u8 - b'0');
                    i += 1;
                }
                let mut format: &[u16] = &[];
                if source.get(i) == Some(&(b'!' as u16)) {
                    let end = source[i + 1..].iter().position(|u| *u == b'!' as u16)?;
                    format = &source[i + 1..i + 1 + end];
                    i += end + 2;
                }
                insert(out, arguments.get(index), format, wide_default)?;
            }
            b'1'..=b'9' => {
                // IGNORE_INSERTS: the insert sequence is copied as is.
                out.push(unit)?;
                out.push(next)?;
            }
            _ => out.push(next)?,
        }
    }
    Some(())
}
/// The message `flags`/`source`/`id` name, expanded: its units, or a
/// Win32 error.
fn message(flags: u32, source: usize, id: u32, arguments: usize, wide: bool) -> Result<Out, u32> {
    let arguments = Arguments {
        pointer: arguments,
        array: flags & ARGUMENT_ARRAY != 0,
    };
    let mut out = Out {
        units: [0; 8192],
        len: 0,
    };
    if flags & FROM_STRING != 0 {
        let mut text = [0u16; 4096];
        let mut n = 0;
        while n < text.len() {
            let unit = if wide {
                unsafe { ((source as *const u16).add(n)).read_unaligned() }
            } else {
                u16::from(unsafe { ((source as *const u8).add(n)).read() })
            };
            if unit == 0 {
                break;
            }
            text[n] = unit;
            n += 1;
        }
        expand(&text[..n], flags, &arguments, wide, &mut out).ok_or(87u32)?;
        return Ok(out);
    }
    if flags & FROM_SYSTEM == 0 {
        // FROM_HMODULE alone: no message tables are read yet.
        return Err(if flags & FROM_HMODULE != 0 { 1813 } else { 87 });
    }
    let text = system_text(id).ok_or(317u32)?;
    let mut units = [0u16; 512];
    let mut n = 0;
    for unit in text.encode_utf16() {
        units[n] = unit;
        n += 1;
    }
    expand(&units[..n], flags, &arguments, wide, &mut out).ok_or(87u32)?;
    Ok(out)
}
fn local_alloc(bytes: usize) -> usize {
    crate::application::LocalAlloc(0, bytes)
}
#[unsafe(no_mangle)]
#[allow(clippy::too_many_arguments)]
pub extern "system" fn FormatMessageW(
    flags: u32,
    source: usize,
    id: u32,
    _language: u32,
    buffer: usize,
    size: u32,
    arguments: usize,
) -> u32 {
    let out = match message(flags, source, id, arguments, true) {
        Ok(out) => out,
        Err(error) => {
            set_error(error);
            return 0;
        }
    };
    let units = &out.units[..out.len];
    let target = if flags & ALLOCATE_BUFFER != 0 {
        let bytes = (units.len() + 1).max(size as usize) * 2;
        let block = local_alloc(bytes);
        if block == 0 {
            set_error(8);
            return 0;
        }
        unsafe { (buffer as *mut usize).write_unaligned(block) };
        block
    } else {
        if units.len() + 1 > size as usize {
            set_error(122);
            return 0;
        }
        buffer
    };
    unsafe {
        core::ptr::copy_nonoverlapping(units.as_ptr(), target as *mut u16, units.len());
        ((target as *mut u16).add(units.len())).write_unaligned(0);
    }
    units.len() as u32
}
/// FormatMessageA: the same message in the ANSI code page (UTF-8 here).
#[unsafe(no_mangle)]
#[allow(clippy::too_many_arguments)]
pub extern "system" fn FormatMessageA(
    flags: u32,
    source: usize,
    id: u32,
    _language: u32,
    buffer: usize,
    size: u32,
    arguments: usize,
) -> u32 {
    let out = match message(flags, source, id, arguments, false) {
        Ok(out) => out,
        Err(error) => {
            set_error(error);
            return 0;
        }
    };
    let mut bytes = [0u8; 16384];
    let mut n = 0;
    for c in char::decode_utf16(out.units[..out.len].iter().copied()) {
        let c = c.unwrap_or('?');
        if n + 4 > bytes.len() {
            break;
        }
        n += c.encode_utf8(&mut bytes[n..]).len();
    }
    let target = if flags & ALLOCATE_BUFFER != 0 {
        let block = local_alloc((n + 1).max(size as usize));
        if block == 0 {
            set_error(8);
            return 0;
        }
        unsafe { (buffer as *mut usize).write_unaligned(block) };
        block
    } else {
        if n + 1 > size as usize {
            set_error(122);
            return 0;
        }
        buffer
    };
    unsafe {
        core::ptr::copy_nonoverlapping(bytes.as_ptr(), target as *mut u8, n);
        ((target as *mut u8).add(n)).write(0);
    }
    n as u32
}
