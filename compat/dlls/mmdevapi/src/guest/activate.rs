//! ActivateAudioInterfaceAsync (P18, Chromium's audio): activation by
//! device interface path. The default render or capture interface
//! ({E6327CAD-...} / {2EEF81BE-...}) or an endpoint ID activates through
//! the device's IMMDevice::Activate; process loopback
//! (`VAD\Process_Loopback`) has no door.audio source and completes with
//! E_NOTIMPL. Activation completes before the call returns: the handler's
//! ActivateCompleted runs on the calling thread with the operation.
use super::*;

const RENDER_INTERFACE: &str = "{E6327CAD-DCEC-4949-AE8A-991E976A79D2}";
const CAPTURE_INTERFACE: &str = "{2EEF81BE-33FA-4800-9670-1CD474972C3F}";
const IID_OPERATION: Guid = guid(
    0x72A2_2D78,
    0xCDE4,
    0x431D,
    [0xB8, 0xCC, 0x84, 0x3A, 0x71, 0x19, 0x9B, 0x6D],
);
const IID_IAGILEOBJECT: Guid = guid(
    0x94EA_2B94,
    0xE9CC,
    0x49E0,
    [0xC0, 0xFF, 0xEE, 0x64, 0xCA, 0x8F, 0x5B, 0x90],
);

#[repr(C)]
struct OperationVtbl {
    query: Fn3,
    add_ref: Fn1,
    release: Fn1,
    result: Fn3,
}
static OPERATION_VTBL: OperationVtbl = OperationVtbl {
    query: operation_query,
    add_ref: obj_add_ref,
    release: obj_release,
    result: operation_result,
};
extern "system" fn operation_query(this: usize, iid: usize, out: usize) -> u32 {
    query(
        door_windows_abi::widen(this),
        door_windows_abi::widen(iid),
        door_windows_abi::widen(out),
        &[IID_OPERATION, IID_IAGILEOBJECT],
        &[],
    )
}
/// GetActivateResult: the activation's HRESULT and, on success, a new
/// reference to the interface.
extern "system" fn operation_result(this: usize, status: usize, out: usize) -> u32 {
    let (status, out) = (door_windows_abi::widen(status), door_windows_abi::widen(out));
    if status == 0 || out == 0 {
        return hr::E_POINTER;
    }
    let Data::Operation(code, iface) = object(door_windows_abi::widen(this)).data else {
        return hr::E_FAIL;
    };
    if iface != 0 {
        com_call1(iface, 1);
    }
    result(write_u32(status, code).and_then(|()| write_pointer(out, iface)))
}
/// Slot `index` of a COM object's table called with no arguments.
pub(super) fn com_call1(object: u64, index: usize) -> u32 {
    let table = read_pointer(object).unwrap_or(0);
    let Ok(f) = read_pointer(table + (index * POINTER) as u64) else {
        return hr::E_POINTER;
    };
    if f == 0 {
        return hr::E_POINTER;
    }
    // SAFETY: a COM interface's IUnknown slot.
    let f: Fn1 = unsafe { core::mem::transmute(f as usize) };
    f(door_windows_abi::narrow(object))
}
fn activate(path: &str, iid: u64) -> (u32, u64) {
    let endpoint = if path.eq_ignore_ascii_case(RENDER_INTERFACE) {
        Some(Endpoint::Render)
    } else if path.eq_ignore_ascii_case(CAPTURE_INTERFACE) {
        Some(Endpoint::Capture)
    } else {
        Endpoint::of_id(path)
    };
    let Some(e) = endpoint else {
        return if path.to_ascii_lowercase().starts_with("vad\\") {
            (hr::E_NOTIMPL, 0)
        } else {
            (hr::E_NOTFOUND, 0)
        };
    };
    if !e.present() {
        return (hr::E_NOTFOUND, 0);
    }
    let device = new_device(e);
    let mut out = 0usize;
    let code = client::activate(
        door_windows_abi::narrow(device),
        door_windows_abi::narrow(iid),
        0x17,
        0,
        &mut out as *mut usize as usize,
    );
    release(device);
    (code, if code == hr::S_OK { out as u64 } else { 0 })
}
#[unsafe(no_mangle)]
pub extern "system" fn ActivateAudioInterfaceAsync(
    path: usize,
    iid: usize,
    _params: usize,
    handler: usize,
    operation: usize,
) -> u32 {
    let (path, iid) = (door_windows_abi::widen(path), door_windows_abi::widen(iid));
    let (handler, operation) = (door_windows_abi::widen(handler), door_windows_abi::widen(operation));
    if path == 0 || iid == 0 || handler == 0 || operation == 0 {
        return hr::E_POINTER;
    }
    let Ok(text) = read_wide(path) else {
        return hr::E_INVALIDARG;
    };
    let (code, iface) = activate(&text, iid);
    let op = new_object(&OPERATION_VTBL as *const _ as u64, 0, Data::Operation(code, iface));
    // The caller's reference, then the one ActivateCompleted borrows.
    add_ref(op);
    if let Err(e) = write_pointer(operation, op) {
        release(op);
        release(op);
        return e;
    }
    // IActivateAudioInterfaceCompletionHandler::ActivateCompleted (slot 3).
    let table = read_pointer(handler).unwrap_or(0);
    if let Ok(f) = read_pointer(table + (3 * POINTER) as u64)
        && f != 0
    {
        // SAFETY: the handler's interface table.
        let f: Fn2 = unsafe { core::mem::transmute(f as usize) };
        f(door_windows_abi::narrow(handler), door_windows_abi::narrow(op));
    }
    release(op);
    hr::S_OK
}
