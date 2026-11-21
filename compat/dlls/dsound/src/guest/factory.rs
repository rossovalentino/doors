//! COM activation (DllGetClassObject): a class factory for each DirectSound
//! class. CoCreateInstance's objects start uninitialized; the application
//! calls Initialize, as DirectSound requires.
use super::*;

const CLSID_DIRECTSOUND: Guid = guid(
    0x47D4_D946,
    0x62E8,
    0x11CF,
    [0x93, 0xBC, 0x44, 0x45, 0x53, 0x54, 0x00, 0x00],
);
const CLSID_DIRECTSOUND8: Guid = guid(
    0x3901_CC3F,
    0x84B5,
    0x4FA4,
    [0xBA, 0x35, 0xAA, 0x81, 0x72, 0xB8, 0xA0, 0x9B],
);
const CLSID_DIRECTSOUNDCAPTURE: Guid = guid(
    0xB021_0780,
    0x89CD,
    0x11D0,
    [0xAF, 0x08, 0x00, 0xA0, 0xC9, 0x25, 0xCD, 0x16],
);
const CLSID_DIRECTSOUNDCAPTURE8: Guid = guid(
    0xE4BC_AC13,
    0x7F99,
    0x4908,
    [0x9A, 0x8E, 0x74, 0xE3, 0xBF, 0x24, 0xB6, 0xE1],
);
const CLSID_DIRECTSOUNDFULLDUPLEX: Guid = guid(
    0xFEA4_300C,
    0x7959,
    0x4147,
    [0xB2, 0x6A, 0x23, 0x77, 0xB9, 0xE7, 0xA9, 0x1D],
);
const IID_ICLASSFACTORY: Guid = guid(1, 0, 0, [0xC0, 0, 0, 0, 0, 0, 0, 0x46]);

#[derive(Clone, Copy, PartialEq, Eq)]
enum Class {
    Render,
    Capture,
    Duplex,
}
/// A factory object: its table, then its class.
#[repr(C)]
struct Factory {
    vtbl: &'static FactoryVtbl,
    class: Class,
}
#[repr(C)]
struct FactoryVtbl {
    query_interface: Fn3,
    add_ref: Fn1,
    release: Fn1,
    create_instance: Fn4,
    lock_server: Fn2i,
}
static FACTORY_VTBL: FactoryVtbl = FactoryVtbl {
    query_interface: f_query_interface,
    add_ref: f_add_ref,
    release: f_release,
    create_instance: f_create_instance,
    lock_server: f_lock_server,
};
static RENDER: Factory = Factory {
    vtbl: &FACTORY_VTBL,
    class: Class::Render,
};
static CAPTURE: Factory = Factory {
    vtbl: &FACTORY_VTBL,
    class: Class::Capture,
};
static DUPLEX: Factory = Factory {
    vtbl: &FACTORY_VTBL,
    class: Class::Duplex,
};
/// Factory references and LockServer locks held by applications.
static HELD: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);

extern "system" fn f_query_interface(this: usize, iid: usize, out: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let iid = door_windows_abi::widen(iid);
    let out = door_windows_abi::widen(out);
    {
        let Ok(iid) = read_guid(iid) else {
            return hr::E_POINTER;
        };
        if out == 0 {
            return hr::E_POINTER;
        }
        if iid == IID_IUNKNOWN || iid == IID_ICLASSFACTORY {
            f_add_ref(door_windows_abi::narrow(this));
            return result(write_pointer(out, this));
        }
        let _ = write_pointer(out, 0);
        hr::DSERR_NOINTERFACE
    }
}
extern "system" fn f_add_ref(_this: usize) -> u32 {
    HELD.fetch_add(1, Ordering::AcqRel) + 1
}
extern "system" fn f_release(_this: usize) -> u32 {
    {
        let left = HELD.load(Ordering::Acquire).saturating_sub(1);
        HELD.store(left, Ordering::Release);
        left
    }
}
extern "system" fn f_create_instance(this: usize, outer: usize, iid: usize, out: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let outer = door_windows_abi::widen(outer);
    let iid = door_windows_abi::widen(iid);
    let out = door_windows_abi::widen(out);
    {
        if out == 0 {
            return hr::E_POINTER;
        }
        let _ = write_pointer(out, 0);
        if outer != 0 {
            return hr::DSERR_NOAGGREGATION;
        }
        // SAFETY: `this` is one of the three static factories.
        let class = unsafe { (*(this as *const Factory)).class };
        let (object, query, release): (u64, Fn3, Fn1) = match class {
            Class::Render => (new_device(false), ds_query_interface, ds_release),
            Class::Capture => (
                capture::new_capture(false),
                capture::c_query_interface,
                capture::c_release,
            ),
            Class::Duplex => (
                capture::new_duplex(),
                capture::f_query_interface,
                capture::f_release,
            ),
        };
        let r = query(
            door_windows_abi::narrow(object),
            door_windows_abi::narrow(iid),
            door_windows_abi::narrow(out),
        );
        release(door_windows_abi::narrow(object));
        r
    }
}
extern "system" fn f_lock_server(_this: usize, lock: i32) -> u32 {
    {
        if lock != 0 {
            HELD.fetch_add(1, Ordering::AcqRel);
        } else {
            f_release(0);
        }
        hr::DS_OK
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn DllGetClassObject(clsid: usize, iid: usize, out: usize) -> u32 {
    let clsid = door_windows_abi::widen(clsid);
    let iid = door_windows_abi::widen(iid);
    let out = door_windows_abi::widen(out);
    {
        if out == 0 {
            return hr::E_POINTER;
        }
        let _ = write_pointer(out, 0);
        let Ok(clsid) = read_guid(clsid) else {
            return hr::DSERR_INVALIDPARAM;
        };
        let factory: &Factory = match clsid {
            CLSID_DIRECTSOUND | CLSID_DIRECTSOUND8 => &RENDER,
            CLSID_DIRECTSOUNDCAPTURE | CLSID_DIRECTSOUNDCAPTURE8 => &CAPTURE,
            CLSID_DIRECTSOUNDFULLDUPLEX => &DUPLEX,
            _ => return hr::CLASS_E_CLASSNOTAVAILABLE,
        };
        f_query_interface(
            door_windows_abi::narrow(factory as *const Factory as u64),
            door_windows_abi::narrow(iid),
            door_windows_abi::narrow(out),
        )
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn DllCanUnloadNow() -> u32 {
    let busy = HELD.load(Ordering::Acquire) != 0
        || OBJECTS.with(|o| !o.devices.is_empty() || !o.buffers.is_empty())
        || capture::live();
    if busy { hr::S_FALSE } else { hr::DS_OK }
}
