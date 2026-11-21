//! Error info: CreateErrorInfo's object (IErrorInfo, and ICreateErrorInfo
//! at its second interface pointer) and the per-thread slot SetErrorInfo
//! fills and GetErrorInfo empties.
use super::*;
use alloc::boxed::Box;
use core::sync::atomic::AtomicU32;

const IID_IUNKNOWN: [u8; 16] = [0, 0, 0, 0, 0, 0, 0, 0, 0xC0, 0, 0, 0, 0, 0, 0, 0x46];
/// {1CF2B120-547D-101B-8E65-08002B2BD119}
const IID_IERRORINFO: [u8; 16] = [
    0x20, 0xB1, 0xF2, 0x1C, 0x7D, 0x54, 0x1B, 0x10, 0x8E, 0x65, 0x08, 0x00, 0x2B, 0x2B, 0xD1, 0x19,
];
/// {22F03340-547D-101B-8E65-08002B2BD119}
const IID_ICREATEERRORINFO: [u8; 16] = [
    0x40, 0x33, 0xF0, 0x22, 0x7D, 0x54, 0x1B, 0x10, 0x8E, 0x65, 0x08, 0x00, 0x2B, 0x2B, 0xD1, 0x19,
];

#[repr(C)]
struct ErrorInfo {
    info: &'static InfoVtbl,
    create: &'static CreateVtbl,
    references: AtomicU32,
    guid: [u8; 16],
    source: Vec<u16>,
    description: Vec<u16>,
    help_file: Vec<u16>,
    help_context: u32,
}
#[repr(C)]
struct InfoVtbl {
    query: extern "system" fn(usize, usize, usize) -> u32,
    add_ref: extern "system" fn(usize) -> u32,
    release: extern "system" fn(usize) -> u32,
    get_guid: extern "system" fn(usize, usize) -> u32,
    get_source: extern "system" fn(usize, usize) -> u32,
    get_description: extern "system" fn(usize, usize) -> u32,
    get_help_file: extern "system" fn(usize, usize) -> u32,
    get_help_context: extern "system" fn(usize, usize) -> u32,
}
#[repr(C)]
struct CreateVtbl {
    query: extern "system" fn(usize, usize, usize) -> u32,
    add_ref: extern "system" fn(usize) -> u32,
    release: extern "system" fn(usize) -> u32,
    set_guid: extern "system" fn(usize, usize) -> u32,
    set_source: extern "system" fn(usize, usize) -> u32,
    set_description: extern "system" fn(usize, usize) -> u32,
    set_help_file: extern "system" fn(usize, usize) -> u32,
    set_help_context: extern "system" fn(usize, u32) -> u32,
}
static INFO_VTBL: InfoVtbl = InfoVtbl {
    query: info_query,
    add_ref: info_add_ref,
    release: info_release,
    get_guid,
    get_source,
    get_description,
    get_help_file,
    get_help_context,
};
static CREATE_VTBL: CreateVtbl = CreateVtbl {
    query: create_query,
    add_ref: create_add_ref,
    release: create_release,
    set_guid,
    set_source,
    set_description,
    set_help_file,
    set_help_context,
};
/// SAFETY: `this` is an ErrorInfo's first interface pointer.
fn object(this: u64) -> &'static mut ErrorInfo {
    unsafe { &mut *(this as *mut ErrorInfo) }
}
extern "system" fn info_query(this: usize, iid: usize, out: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let iid = door_windows_abi::widen(iid);
    let out = door_windows_abi::widen(out);
    {
        let Ok(g) = read_n::<16>(iid) else {
            return hr::E_POINTER;
        };
        let pointer = if g == IID_IUNKNOWN || g == IID_IERRORINFO {
            this
        } else if g == IID_ICREATEERRORINFO {
            this + POINTER as u64
        } else {
            let _ = write_pointer(out, 0);
            return hr::E_NOINTERFACE;
        };
        info_add_ref(door_windows_abi::narrow(this));
        result(write_pointer(out, pointer))
    }
}
extern "system" fn info_add_ref(this: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    object(this).references.fetch_add(1, Ordering::AcqRel) + 1
}
extern "system" fn info_release(this: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    {
        let left = object(this).references.fetch_sub(1, Ordering::AcqRel) - 1;
        if left == 0 {
            // SAFETY: the last reference; the object came from Box::into_raw.
            drop(unsafe { Box::from_raw(this as *mut ErrorInfo) });
        }
        left
    }
}
extern "system" fn create_query(this: usize, iid: usize, out: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let iid = door_windows_abi::widen(iid);
    let out = door_windows_abi::widen(out);
    {
        info_query(
            door_windows_abi::narrow(this - POINTER as u64),
            door_windows_abi::narrow(iid),
            door_windows_abi::narrow(out),
        )
    }
}
extern "system" fn create_add_ref(this: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    info_add_ref(door_windows_abi::narrow(this - POINTER as u64))
}
extern "system" fn create_release(this: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    info_release(door_windows_abi::narrow(this - POINTER as u64))
}
extern "system" fn get_guid(this: usize, out: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let out = door_windows_abi::widen(out);
    result(write(out, &object(this).guid))
}
fn give_text(units: &[u16], out: u64) -> u32 {
    let s = if units.is_empty() {
        0
    } else {
        bstr_from(units)
    };
    result(write_pointer(out, s))
}
extern "system" fn get_source(this: usize, out: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let out = door_windows_abi::widen(out);
    give_text(&object(this).source.clone(), out)
}
extern "system" fn get_description(this: usize, out: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let out = door_windows_abi::widen(out);
    give_text(&object(this).description.clone(), out)
}
extern "system" fn get_help_file(this: usize, out: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let out = door_windows_abi::widen(out);
    give_text(&object(this).help_file.clone(), out)
}
extern "system" fn get_help_context(this: usize, out: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let out = door_windows_abi::widen(out);
    result(write_u32(out, object(this).help_context))
}
fn text_or_empty(p: u64) -> Result<Vec<u16>, u32> {
    if p == 0 { Ok(Vec::new()) } else { read_wide(p) }
}
extern "system" fn set_guid(this: usize, guid: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let guid = door_windows_abi::widen(guid);
    {
        match read_n::<16>(guid) {
            Ok(g) => {
                object(this - POINTER as u64).guid = g;
                hr::S_OK
            }
            Err(e) => e,
        }
    }
}
extern "system" fn set_source(this: usize, text: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let text = door_windows_abi::widen(text);
    {
        match text_or_empty(text) {
            Ok(t) => {
                object(this - POINTER as u64).source = t;
                hr::S_OK
            }
            Err(e) => e,
        }
    }
}
extern "system" fn set_description(this: usize, text: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let text = door_windows_abi::widen(text);
    {
        match text_or_empty(text) {
            Ok(t) => {
                object(this - POINTER as u64).description = t;
                hr::S_OK
            }
            Err(e) => e,
        }
    }
}
extern "system" fn set_help_file(this: usize, text: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let text = door_windows_abi::widen(text);
    {
        match text_or_empty(text) {
            Ok(t) => {
                object(this - POINTER as u64).help_file = t;
                hr::S_OK
            }
            Err(e) => e,
        }
    }
}
extern "system" fn set_help_context(this: usize, context: u32) -> u32 {
    let this = door_windows_abi::widen(this);
    {
        object(this - POINTER as u64).help_context = context;
        hr::S_OK
    }
}
/// CreateErrorInfo: an ICreateErrorInfo (QueryInterface gives IErrorInfo).
#[unsafe(no_mangle)]
pub extern "system" fn CreateErrorInfo(out: usize) -> u32 {
    let out = door_windows_abi::widen(out);
    {
        if out == 0 {
            return hr::E_INVALIDARG;
        }
        let o = Box::new(ErrorInfo {
            info: &INFO_VTBL,
            create: &CREATE_VTBL,
            references: AtomicU32::new(1),
            guid: [0; 16],
            source: Vec::new(),
            description: Vec::new(),
            help_file: Vec::new(),
            help_context: 0,
        });
        let this = Box::into_raw(o) as u64;
        result(write_pointer(out, this + POINTER as u64))
    }
}
/// The thread's error object (the caller's reference).
static SLOTS: Lock<Vec<(u32, u64)>> = Lock::new(Vec::new());
#[unsafe(no_mangle)]
pub extern "system" fn SetErrorInfo(_reserved: u32, info: usize) -> u32 {
    let info = door_windows_abi::widen(info);
    {
        add_ref(info);
        let me = thread();
        let old = SLOTS.with(|s| {
            let old = s.iter().position(|(t, _)| *t == me).map(|i| s.remove(i).1);
            if info != 0 {
                s.push((me, info));
            }
            old
        });
        if let Some(o) = old {
            release(o);
        }
        hr::S_OK
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn GetErrorInfo(_reserved: u32, out: usize) -> u32 {
    let out = door_windows_abi::widen(out);
    {
        let me = thread();
        let taken = SLOTS.with(|s| s.iter().position(|(t, _)| *t == me).map(|i| s.remove(i).1));
        match taken {
            Some(p) => result(write_pointer(out, p)),
            None => {
                let _ = write_pointer(out, 0);
                hr::S_FALSE
            }
        }
    }
}
