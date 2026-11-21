//! shlwapi.dll exports.
use crate::*;
use alloc::{string::String, vec::Vec};

#[global_allocator]
static HEAP: door_windows_abi::allocator::ProcessHeap = door_windows_abi::allocator::ProcessHeap;

#[link(name = "advapi32", kind = "dylib")]
unsafe extern "system" {
    fn RegGetValueW(
        key: usize,
        sub: usize,
        value: usize,
        flags: u32,
        kind: usize,
        data: usize,
        size: usize,
    ) -> u32;
}
#[link(name = "kernel32", kind = "dylib")]
unsafe extern "system" {
    fn GetFileAttributesW(path: usize) -> u32;
    fn GetProcessHeap() -> usize;
    fn HeapAlloc(heap: usize, flags: u32, bytes: usize) -> usize;
}
#[link(name = "ole32", kind = "dylib")]
unsafe extern "system" {
    fn CreateStreamOnHGlobal(handle: usize, delete: i32, out: usize) -> u32;
    fn CoTaskMemAlloc(bytes: usize) -> usize;
}

const S_OK: u32 = 0;
const S_FALSE: u32 = 1;
const E_POINTER: u32 = 0x8000_4003;
const E_NOINTERFACE: u32 = 0x8000_4002;
const E_INVALIDARG: u32 = 0x8007_0057;
const E_OUTOFMEMORY: u32 = 0x8007_000E;
/// HRESULT_FROM_WIN32(ERROR_NO_ASSOCIATION).
const NO_ASSOCIATION: u32 = 0x8007_0483;
const POINTER: usize = core::mem::size_of::<usize>();

/// A NUL-terminated UTF-16 string at `p` (at most `max` units).
fn units(p: usize, max: usize) -> Vec<u16> {
    let mut out = Vec::new();
    if p == 0 {
        return out;
    }
    // SAFETY: the caller's string, read up to its terminator.
    while out.len() < max {
        let u = unsafe { (p as *const u16).add(out.len()).read_unaligned() };
        if u == 0 {
            break;
        }
        out.push(u);
    }
    out
}
fn units_a(p: usize, max: usize) -> Vec<u8> {
    let mut out = Vec::new();
    if p == 0 {
        return out;
    }
    while out.len() < max {
        // SAFETY: as `units`.
        let b = unsafe { (p as *const u8).add(out.len()).read() };
        if b == 0 {
            break;
        }
        out.push(b);
    }
    out
}

// ---- paths ------------------------------------------------------------------------
#[unsafe(no_mangle)]
pub extern "system" fn PathFindFileNameW(path: usize) -> usize {
    if path == 0 {
        return 0;
    }
    path + 2 * file_name(&units(path, 32767))
}
#[unsafe(no_mangle)]
pub extern "system" fn PathFindFileNameA(path: usize) -> usize {
    if path == 0 {
        return 0;
    }
    let wide: Vec<u16> = units_a(path, 32767).iter().map(|&b| u16::from(b)).collect();
    path + file_name(&wide)
}
#[unsafe(no_mangle)]
pub extern "system" fn PathFindExtensionW(path: usize) -> usize {
    if path == 0 {
        return 0;
    }
    path + 2 * extension(&units(path, 32767))
}
#[unsafe(no_mangle)]
pub extern "system" fn PathFindExtensionA(path: usize) -> usize {
    if path == 0 {
        return 0;
    }
    let wide: Vec<u16> = units_a(path, 32767).iter().map(|&b| u16::from(b)).collect();
    path + extension(&wide)
}
#[unsafe(no_mangle)]
pub extern "system" fn PathRemoveExtensionW(path: usize) {
    let at = PathFindExtensionW(path);
    if at != 0 {
        // SAFETY: inside the caller's string.
        unsafe { (at as *mut u16).write_unaligned(0) };
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn PathMatchSpecW(path: usize, spec: usize) -> i32 {
    if path == 0 || spec == 0 {
        return 0;
    }
    i32::from(matches(&units(path, 32767), &units(spec, 32767)))
}
#[unsafe(no_mangle)]
pub extern "system" fn PathFileExistsW(path: usize) -> i32 {
    i32::from(path != 0 && unsafe { GetFileAttributesW(path) } != u32::MAX)
}
#[unsafe(no_mangle)]
pub extern "system" fn PathIsDirectoryW(path: usize) -> u32 {
    let a = if path == 0 {
        u32::MAX
    } else {
        unsafe { GetFileAttributesW(path) }
    };
    // FILE_ATTRIBUTE_DIRECTORY, as Windows returns it.
    if a != u32::MAX && a & 0x10 != 0 {
        0x10
    } else {
        0
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn PathIsRelativeW(path: usize) -> i32 {
    let p = units(path, 32767);
    let absolute = p.first() == Some(&(b'\\' as u16)) || (p.len() >= 2 && p[1] == u16::from(b':'));
    i32::from(!absolute)
}
/// PathRemoveFileSpecW: cut the last component (keeping a root's `\`).
#[unsafe(no_mangle)]
pub extern "system" fn PathRemoveFileSpecW(path: usize) -> i32 {
    let p = units(path, 32767);
    let name = file_name(&p);
    if name == 0 {
        return 0;
    }
    let mut cut = name;
    // "C:\x" keeps "C:\"; "\x" keeps "\"; "a\b" becomes "a".
    if cut > 0
        && p[cut - 1] == u16::from(b'\\')
        && !(cut == 3 && p[1] == u16::from(b':'))
        && cut != 1
    {
        cut -= 1;
    }
    // SAFETY: inside the caller's string.
    unsafe { (path as *mut u16).add(cut).write_unaligned(0) };
    1
}
#[unsafe(no_mangle)]
pub extern "system" fn PathAddBackslashW(path: usize) -> usize {
    let p = units(path, 32767);
    if p.len() >= 259 {
        return 0;
    }
    let end = path + 2 * p.len();
    if p.last() != Some(&(b'\\' as u16)) && !p.is_empty() {
        // SAFETY: MAX_PATH buffer, as the API requires.
        unsafe {
            (end as *mut u16).write_unaligned(u16::from(b'\\'));
            (end as *mut u16).add(1).write_unaligned(0);
        }
        return end + 2;
    }
    end
}
#[unsafe(no_mangle)]
pub extern "system" fn PathAppendW(path: usize, more: usize) -> i32 {
    let mut p = units(path, 32767);
    let m = units(more, 32767);
    let m: Vec<u16> = m
        .iter()
        .copied()
        .skip_while(|&u| u == u16::from(b'\\'))
        .collect();
    if !p.is_empty() && p.last() != Some(&(b'\\' as u16)) && !m.is_empty() {
        p.push(u16::from(b'\\'));
    }
    p.extend_from_slice(&m);
    if p.len() >= 260 {
        return 0;
    }
    p.push(0);
    // SAFETY: MAX_PATH buffer, as the API requires.
    unsafe { core::ptr::copy_nonoverlapping(p.as_ptr(), path as *mut u16, p.len()) };
    1
}
#[unsafe(no_mangle)]
pub extern "system" fn PathCombineW(out: usize, directory: usize, file: usize) -> usize {
    if out == 0 {
        return 0;
    }
    let f = units(file, 32767);
    let absolute = f.first() == Some(&(b'\\' as u16)) || (f.len() >= 2 && f[1] == u16::from(b':'));
    let mut p = if absolute || directory == 0 {
        Vec::new()
    } else {
        units(directory, 32767)
    };
    if !p.is_empty() && p.last() != Some(&(b'\\' as u16)) && !f.is_empty() {
        p.push(u16::from(b'\\'));
    }
    p.extend_from_slice(&f);
    if p.len() >= 260 {
        // SAFETY: MAX_PATH buffer.
        unsafe { (out as *mut u16).write_unaligned(0) };
        return 0;
    }
    p.push(0);
    // SAFETY: MAX_PATH buffer, as the API requires.
    unsafe { core::ptr::copy(p.as_ptr(), out as *mut u16, p.len()) };
    out
}
#[unsafe(no_mangle)]
pub extern "system" fn StrCmpIW(a: usize, b: usize) -> i32 {
    let fold = |s: Vec<u16>| -> String { String::from_utf16_lossy(&s).to_lowercase() };
    match fold(units(a, 32767)).cmp(&fold(units(b, 32767))) {
        core::cmp::Ordering::Less => -1,
        core::cmp::Ordering::Equal => 0,
        core::cmp::Ordering::Greater => 1,
    }
}
/// SHStrDupW: a CoTaskMemAlloc copy.
#[unsafe(no_mangle)]
pub extern "system" fn SHStrDupW(source: usize, out: usize) -> u32 {
    if source == 0 || out == 0 {
        return E_INVALIDARG;
    }
    let mut s = units(source, 32767);
    s.push(0);
    let block = unsafe { CoTaskMemAlloc(2 * s.len()) };
    if block == 0 {
        return E_OUTOFMEMORY;
    }
    // SAFETY: a fresh block and the caller's out-pointer.
    unsafe {
        core::ptr::copy_nonoverlapping(s.as_ptr(), block as *mut u16, s.len());
        (out as *mut usize).write_unaligned(block);
    }
    S_OK
}

// ---- associations ---------------------------------------------------------------------
const HKEY_CLASSES_ROOT: usize = 0x8000_0000_u32 as usize;
fn class_value(sub: &str) -> Option<Vec<u16>> {
    let key: Vec<u16> = sub.encode_utf16().chain([0]).collect();
    let mut buffer = [0u16; 1024];
    let mut size = (buffer.len() * 2) as u32;
    // RRF_RT_REG_SZ | RRF_RT_REG_EXPAND_SZ.
    let r = unsafe {
        RegGetValueW(
            HKEY_CLASSES_ROOT,
            key.as_ptr() as usize,
            0,
            0x2 | 0x4,
            0,
            buffer.as_mut_ptr() as usize,
            &mut size as *mut u32 as usize,
        )
    };
    if r != 0 {
        return None;
    }
    let n = (size as usize / 2).min(buffer.len());
    let end = buffer[..n].iter().position(|&u| u == 0).unwrap_or(n);
    (end > 0).then(|| buffer[..end].to_vec())
}
/// The first token of a command line (quoted or up to a space).
fn executable(command: &str) -> String {
    let c = command.trim_start();
    match c.strip_prefix('"') {
        Some(rest) => String::from(rest.split('"').next().unwrap_or("")),
        None => String::from(c.split(' ').next().unwrap_or("")),
    }
}
/// AssocQueryStringW over HKCR: ASSOCSTR_COMMAND 1, ASSOCSTR_EXECUTABLE 2,
/// ASSOCSTR_FRIENDLYDOCNAME 3, ASSOCSTR_FRIENDLYAPPNAME 4,
/// ASSOCSTR_DEFAULTICON 15. `association` is an extension or a ProgID;
/// `extra` the verb (open by default).
#[unsafe(no_mangle)]
pub extern "system" fn AssocQueryStringW(
    _flags: u32,
    kind: u32,
    association: usize,
    extra: usize,
    out: usize,
    length: usize,
) -> u32 {
    if association == 0 || length == 0 {
        return E_INVALIDARG;
    }
    let name = String::from_utf16_lossy(&units(association, 1024));
    let prog_id = if name.starts_with('.') {
        match class_value(&name) {
            Some(id) => String::from_utf16_lossy(&id),
            None => return NO_ASSOCIATION,
        }
    } else {
        name
    };
    let verb = if extra == 0 {
        String::from("open")
    } else {
        String::from_utf16_lossy(&units(extra, 256))
    };
    let command = || {
        class_value(&alloc::format!(r"{prog_id}\shell\{verb}\command"))
            .map(|c| String::from_utf16_lossy(&c))
    };
    let text: String = match kind {
        1 => match command() {
            Some(c) => c,
            None => return NO_ASSOCIATION,
        },
        2 => match command() {
            Some(c) => executable(&c),
            None => return NO_ASSOCIATION,
        },
        3 => match class_value(&prog_id) {
            Some(n) => String::from_utf16_lossy(&n),
            None => return NO_ASSOCIATION,
        },
        4 => match command() {
            Some(c) => {
                let exe = executable(&c);
                let file = exe.rsplit('\\').next().unwrap_or(&exe);
                String::from(file.rsplit_once('.').map_or(file, |(stem, _)| stem))
            }
            None => return NO_ASSOCIATION,
        },
        15 => match class_value(&alloc::format!(r"{prog_id}\DefaultIcon")) {
            Some(n) => String::from_utf16_lossy(&n),
            None => return NO_ASSOCIATION,
        },
        _ => return NO_ASSOCIATION,
    };
    let w: Vec<u16> = text.encode_utf16().chain([0]).collect();
    // SAFETY: the caller's in/out character count and buffer.
    let capacity = unsafe { (length as *const u32).read_unaligned() } as usize;
    unsafe { (length as *mut u32).write_unaligned(w.len() as u32) };
    if out == 0 {
        return S_FALSE;
    }
    if capacity < w.len() {
        return E_POINTER;
    }
    unsafe { core::ptr::copy_nonoverlapping(w.as_ptr(), out as *mut u16, w.len()) };
    S_OK
}

// ---- COM helpers ---------------------------------------------------------------------
/// QISearch (ordinal 219): QueryInterface over a QITAB ({IID*, offset}
/// entries, a NULL IID last); IUnknown answers with the first entry.
#[unsafe(no_mangle)]
pub extern "system" fn QISearch(this: usize, table: usize, iid: usize, out: usize) -> u32 {
    if out == 0 {
        return E_POINTER;
    }
    // SAFETY: the caller's out-pointer.
    unsafe { (out as *mut usize).write_unaligned(0) };
    if this == 0 || table == 0 || iid == 0 {
        return E_POINTER;
    }
    const IUNKNOWN: [u8; 16] = [0, 0, 0, 0, 0, 0, 0, 0, 0xC0, 0, 0, 0, 0, 0, 0, 0x46];
    // SAFETY: the caller's IID.
    let wanted = unsafe { (iid as *const [u8; 16]).read_unaligned() };
    let entry = 2 * POINTER;
    for i in 0..256 {
        // SAFETY: the caller's QITAB, ended by a NULL IID.
        let (piid, offset) = unsafe {
            let at = table + i * entry;
            (
                (at as *const usize).read_unaligned(),
                ((at + POINTER) as *const u32).read_unaligned(),
            )
        };
        if piid == 0 {
            break;
        }
        let candidate = unsafe { (piid as *const [u8; 16]).read_unaligned() };
        if candidate == wanted || (i == 0 && wanted == IUNKNOWN) {
            let interface = this + offset as usize;
            // SAFETY: IUnknown::AddRef of the caller's object.
            unsafe {
                let vtbl = (interface as *const usize).read_unaligned();
                let add_ref: extern "system" fn(usize) -> u32 =
                    core::mem::transmute(((vtbl + POINTER) as *const usize).read_unaligned());
                add_ref(interface);
                (out as *mut usize).write_unaligned(interface);
            }
            return S_OK;
        }
    }
    E_NOINTERFACE
}
/// SHCreateMemStream (ordinal 12): an IStream over a copy of `bytes`
/// (NULL with no memory).
#[unsafe(no_mangle)]
pub extern "system" fn SHCreateMemStream(data: usize, bytes: u32) -> usize {
    let n = if data == 0 { 0 } else { bytes as usize };
    let block = unsafe { HeapAlloc(GetProcessHeap(), 0, n.max(1)) };
    if block == 0 {
        return 0;
    }
    if n > 0 {
        // SAFETY: the caller's bytes and a fresh block of `n`.
        unsafe { core::ptr::copy_nonoverlapping(data as *const u8, block as *mut u8, n) };
    }
    let mut stream = 0usize;
    if unsafe { CreateStreamOnHGlobal(block, 1, &mut stream as *mut usize as usize) } != S_OK {
        return 0;
    }
    // The block may be larger than asked: IStream::SetSize (slot 6) to `n`.
    // SAFETY: the new stream's interface table.
    unsafe {
        let vtbl = (stream as *const usize).read_unaligned();
        let set_size: extern "system" fn(usize, u64) -> u32 =
            core::mem::transmute(((vtbl + 6 * POINTER) as *const usize).read_unaligned());
        set_size(stream, n as u64);
    }
    stream
}
