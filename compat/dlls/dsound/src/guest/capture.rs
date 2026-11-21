//! DirectSoundCapture: IDirectSoundCapture8 and its buffers over door.audio
//! capture streams. Each buffer owns a capture ring (a section this process
//! maps for good, as winmm's waveIn does); the worker converts what the ring
//! holds to the buffer's format with winmm's recorder and writes it at the
//! capture cursor, wrapping when the buffer loops and stopping at its end
//! when it does not. Notifications are set as the cursor passes them.
//! Privacy & security > Microphone decides at CreateCaptureBuffer, where
//! Windows opens the capture stream.
use super::*;
use door_service_protocol::audio::capture as ring;
use winmm::{Recorder, Wave};

const IID_IDIRECTSOUNDCAPTURE: Guid = guid(
    0xB021_0781,
    0x89CD,
    0x11D0,
    [0xAF, 0x08, 0x00, 0xA0, 0xC9, 0x25, 0xCD, 0x16],
);
const IID_IDIRECTSOUNDCAPTUREBUFFER: Guid = guid(
    0xB021_0782,
    0x89CD,
    0x11D0,
    [0xAF, 0x08, 0x00, 0xA0, 0xC9, 0x25, 0xCD, 0x16],
);
const IID_IDIRECTSOUNDCAPTUREBUFFER8: Guid = guid(
    0x0099_0DF4,
    0x0DBB,
    0x4872,
    [0x83, 0x3E, 0x6D, 0x30, 0x3E, 0x80, 0xAE, 0xB6],
);
const IID_IDIRECTSOUNDFULLDUPLEX: Guid = guid(
    0xEDCB_4C7A,
    0xDAAB,
    0x4216,
    [0xA4, 0x2E, 0x6C, 0x50, 0x59, 0x6D, 0xDC, 0x1D],
);
const DSDEVID_DEFAULT_CAPTURE: Guid = guid(
    0xDEF0_0001,
    0x9C6D,
    0x47ED,
    [0xAA, 0xF1, 0x4D, 0xDA, 0x8F, 0x2B, 0x5C, 0x03],
);
const DSDEVID_DEFAULT_VOICE_CAPTURE: Guid = guid(
    0xDEF0_0003,
    0x9C6D,
    0x47ED,
    [0xAA, 0xF1, 0x4D, 0xDA, 0x8F, 0x2B, 0x5C, 0x03],
);
/// The Doors microphone's device GUID.
pub(super) const MICROPHONE: Guid = guid(
    0xD002_5D01,
    0x5EA4,
    0x4A6B,
    [0x9C, 0x11, 0x2B, 0x6E, 0x15, 0x50, 0x00, 0x02],
);
const MICROPHONE_NAME: &str = "Microphone (VirtIO Sound)";
const DSCBSTART_LOOPING: u32 = 1;
const DSCBSTATUS_CAPTURING: u32 = 1;
const DSCBSTATUS_LOOPING: u32 = 2;
const DSCBLOCK_ENTIREBUFFER: u32 = 1;
const DSCBCAPS_CTRLFX: u32 = 0x200;
const DSCBCAPS_WAVEMAPPED: u32 = 0x8000_0000;
const E_ACCESSDENIED: u32 = 0x8007_0005;
/// Ring capacity: two seconds of captured sound.
const RING_BYTES: usize = 200 * ring::PERIOD_BYTES;

#[link(name = "ntdll", kind = "dylib")]
unsafe extern "system" {
    fn DoorGetModuleFileNameW(module: u64, buffer: u64, capacity: u32) -> u32;
}

struct Device {
    id: u32,
    refs: u32,
    /// False for an object CoCreateInstance made, until Initialize.
    initialized: bool,
}
struct Buffer {
    id: u32,
    device: u32,
    refs: u32,
    iface: u64,
    notify_iface: u64,
    flags: u32,
    format: Format,
    data: Vec<u8>,
    /// Capture cursor (bytes into `data`).
    cursor: usize,
    capturing: bool,
    looping: bool,
    notify: Vec<(u32, u64)>,
    capture: u32,
    section: u64,
    ring: u64,
    mapped: u64,
    recorder: Recorder,
    converted: Vec<u8>,
}
struct Objects {
    devices: Vec<Device>,
    buffers: Vec<Buffer>,
}
static CAPTURE: Lock<Objects> = Lock::new(Objects {
    devices: Vec::new(),
    buffers: Vec::new(),
});
fn next_id() -> u32 {
    OBJECTS.with(|o| {
        let id = o.next;
        o.next += 1;
        id
    })
}

fn capture_present() -> bool {
    audio(|a| a.status().is_ok_and(|s| s.present && s.capture)).unwrap_or(false)
}
fn ring_word(base: u64, at: usize) -> &'static core::sync::atomic::AtomicU32 {
    // SAFETY: the ring stays mapped while its buffer exists; fields aligned.
    unsafe { &*((base + at as u64) as *const core::sync::atomic::AtomicU32) }
}
fn ring_count(base: u64, at: usize) -> &'static core::sync::atomic::AtomicU64 {
    // SAFETY: as ring_word.
    unsafe { &*((base + at as u64) as *const core::sync::atomic::AtomicU64) }
}
/// A ring section mapped here with its header prepared: (section, address,
/// mapped bytes).
fn create_ring() -> Option<(u64, u64, u64)> {
    let mapped = ((ring::HEADER + RING_BYTES) as u64).div_ceil(4096) * 4096;
    let address = unsafe { DoorReserveSectionRange(mapped) };
    if address == 0 {
        return None;
    }
    let Ok(section) =
        door_sys::section_create(mapped, door_abi::rights::ALL & !door_abi::rights::WAIT)
    else {
        unsafe { DoorReleaseSectionRange(address, mapped) };
        return None;
    };
    // SAFETY: the reserved range is this mapping's alone.
    if unsafe {
        door_sys::section_map(
            section,
            address,
            door_abi::rights::READ | door_abi::rights::WRITE,
        )
    }
    .is_err()
    {
        let _ = door_sys::close(section);
        unsafe { DoorReleaseSectionRange(address, mapped) };
        return None;
    }
    ring_word(address, ring::MAGIC).store(ring::MAGIC_VALUE, Ordering::Relaxed);
    ring_word(address, ring::CAPACITY).store(RING_BYTES as u32, Ordering::Release);
    Some((section, address, mapped))
}
fn release_ring(section: u64, address: u64, mapped: u64) {
    // SAFETY: no reference into the ring outlives its buffer.
    if unsafe { door_sys::section_unmap(address, mapped) }.is_ok() {
        unsafe { DoorReleaseSectionRange(address, mapped) };
    }
    let _ = door_sys::close(section);
}
/// The process executable file name (the Microphone page lists it).
fn executable_name() -> alloc::string::String {
    let mut units = [0u16; 260];
    let n = unsafe { DoorGetModuleFileNameW(0, units.as_mut_ptr() as u64, units.len() as u32) }
        as usize;
    let path = alloc::string::String::from_utf16_lossy(&units[..n.min(units.len())]);
    let name = path.rsplit(['\\', '/']).next().unwrap_or("");
    let mut end = name.len().min(door_service_protocol::audio::NAME_BYTES);
    while !name.is_char_boundary(end) {
        end -= 1;
    }
    alloc::string::String::from(&name[..end])
}

// ---- the worker's part ---------------------------------------------------------
/// Moves what every capture ring holds into its buffer; the events to set.
pub(super) fn pass() -> Vec<u64> {
    let mut events = Vec::new();
    CAPTURE.with(|o| {
        for b in o.buffers.iter_mut() {
            let written = ring_count(b.ring, ring::WRITE).load(Ordering::Acquire);
            let mut read = ring_count(b.ring, ring::READ).load(Ordering::Relaxed);
            if b.capturing {
                let base = b.ring + ring::HEADER as u64;
                while read < written {
                    let at = (read % RING_BYTES as u64) as usize;
                    let take = ((written - read) as usize)
                        .min(RING_BYTES - at)
                        .min(ring::PERIOD_BYTES * 10);
                    // SAFETY: the producer finished these bytes (Acquire above).
                    let bytes = unsafe {
                        core::slice::from_raw_parts((base + at as u64) as *const u8, take)
                    };
                    b.recorder.push(bytes, &mut b.converted);
                    read += take as u64;
                }
                let block = usize::from(b.format.block);
                let whole = b.converted.len() - b.converted.len() % block;
                let mut taken = 0;
                while taken < whole && b.capturing {
                    let before = b.cursor;
                    let room = b.data.len() - b.cursor;
                    let n = (whole - taken).min(room);
                    b.data[b.cursor..b.cursor + n].copy_from_slice(&b.converted[taken..taken + n]);
                    taken += n;
                    b.cursor += n;
                    if b.cursor == b.data.len() {
                        b.cursor = 0;
                        if !b.looping {
                            b.capturing = false;
                            events.extend(
                                b.notify
                                    .iter()
                                    .filter(|(o, _)| *o == DSBPN_OFFSETSTOP)
                                    .map(|(_, e)| *e),
                            );
                        }
                    }
                    // Offsets the cursor passed (an offset at the cursor's
                    // end counts as passed: its byte has been written).
                    let to = if b.cursor == 0 {
                        b.data.len()
                    } else {
                        b.cursor
                    };
                    events.extend(
                        b.notify
                            .iter()
                            .filter(|(o, _)| {
                                *o != DSBPN_OFFSETSTOP
                                    && (*o as usize) >= before
                                    && (*o as usize) < to
                            })
                            .map(|(_, e)| *e),
                    );
                }
                b.converted.drain(..taken.min(b.converted.len()));
                if !b.capturing {
                    b.converted.clear();
                }
            }
            ring_count(b.ring, ring::READ).store(written, Ordering::Release);
        }
    });
    events
}

// ---- IDirectSoundCapture8 --------------------------------------------------------
#[repr(C)]
struct CaptureVtbl {
    query_interface: Fn3,
    add_ref: Fn1,
    release: Fn1,
    create_capture_buffer: Fn4,
    get_caps: Fn2,
    initialize: Fn2,
}
static CAPTURE_VTBL: CaptureVtbl = CaptureVtbl {
    query_interface: c_query_interface,
    add_ref: c_add_ref,
    release: c_release,
    create_capture_buffer: c_create_capture_buffer,
    get_caps: c_get_caps,
    initialize: c_initialize,
};
fn with_capture_object<R>(
    this: u64,
    f: impl FnOnce(&mut Device) -> Result<R, u32>,
) -> Result<R, u32> {
    let id = iface_id(this).ok_or(hr::E_POINTER)?;
    CAPTURE.with(|o| {
        o.devices
            .iter_mut()
            .find(|d| d.id == id)
            .map_or(Err(hr::DSERR_INVALIDCALL), f)
    })
}
/// An initialized capture device.
fn with_capture<R>(this: u64, f: impl FnOnce(&mut Device) -> Result<R, u32>) -> Result<R, u32> {
    with_capture_object(this, |d| {
        if d.initialized {
            f(d)
        } else {
            Err(hr::DSERR_UNINITIALIZED)
        }
    })
}
pub(super) extern "system" fn c_query_interface(this: usize, iid: usize, out: usize) -> u32 {
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
        if [IID_IUNKNOWN, IID_IDIRECTSOUNDCAPTURE].contains(&iid)
            && c_add_ref(door_windows_abi::narrow(this)) != 0
        {
            return result(write_pointer(out, this));
        }
        let _ = write_pointer(out, 0);
        hr::DSERR_NOINTERFACE
    }
}
extern "system" fn c_add_ref(this: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    {
        with_capture_object(this, |d| {
            d.refs += 1;
            Ok(d.refs)
        })
        .unwrap_or(0)
    }
}
pub(super) extern "system" fn c_release(this: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    {
        let Some(id) = iface_id(this) else { return 0 };
        let (left, closed) = CAPTURE.with(|o| {
            let Some(at) = o.devices.iter().position(|d| d.id == id) else {
                return (0, Vec::new());
            };
            o.devices[at].refs -= 1;
            let left = o.devices[at].refs;
            if left != 0 {
                return (left, Vec::new());
            }
            o.devices.remove(at);
            let mut closed = Vec::new();
            o.buffers.retain(|b| {
                if b.device == id {
                    closed.push((b.capture, b.section, b.ring, b.mapped));
                    false
                } else {
                    true
                }
            });
            (0, closed)
        });
        for (capture, section, address, mapped) in closed {
            audio(|a| a.capture_close(capture));
            release_ring(section, address, mapped);
        }
        left
    }
}
/// DSCBUFFERDESC: dwSize (24, or 40 with the DirectX 8 effect fields),
/// dwFlags, dwBufferBytes, dwReserved, lpwfxFormat.
extern "system" fn c_create_capture_buffer(
    this: usize,
    desc: usize,
    out: usize,
    outer: usize,
) -> u32 {
    let this = door_windows_abi::widen(this);
    let desc = door_windows_abi::widen(desc);
    let out = door_windows_abi::widen(out);
    let outer = door_windows_abi::widen(outer);
    {
        if outer != 0 {
            return hr::DSERR_NOAGGREGATION;
        }
        if out == 0 {
            return hr::DSERR_INVALIDPARAM;
        }
        let _ = write_pointer(out, 0);
        let Some(device) = iface_id(this) else {
            return hr::E_POINTER;
        };
        if let Err(e) = with_capture(this, |_| Ok(())) {
            return e;
        }
        let mut d = [0u8; 24];
        if read(desc, &mut d).is_err() {
            return hr::DSERR_INVALIDPARAM;
        }
        let word = |at: usize| u32::from_le_bytes(d[at..at + 4].try_into().unwrap());
        let (size, flags, bytes) = (word(0), word(4), word(8));
        let format_ptr = super::format_pointer(&d);
        if (size != super::DESC1 && size != super::CAPTURE_DESC)
            || flags & !(DSCBCAPS_CTRLFX | DSCBCAPS_WAVEMAPPED) != 0
        {
            return hr::DSERR_INVALIDPARAM;
        }
        if flags & DSCBCAPS_CTRLFX != 0 {
            // Capture effects (echo cancellation, noise suppression) are not
            // offered.
            return hr::DSERR_CONTROLUNAVAIL;
        }
        let mut f = [0u8; 40];
        if read(format_ptr, &mut f[..18]).is_err() {
            return hr::DSERR_INVALIDPARAM;
        }
        if u16::from_le_bytes([f[0], f[1]]) == WAVE_FORMAT_EXTENSIBLE
            && read(format_ptr + 18, &mut f[18..40]).is_err()
        {
            return hr::DSERR_INVALIDPARAM;
        }
        let (Some(format), Some(wave)) = (Format::parse(&f), Wave::parse(&f)) else {
            return Format::refusal(&f);
        };
        if !(DSBSIZE_MIN..=DSBSIZE_MAX).contains(&bytes)
            || !(bytes as usize).is_multiple_of(usize::from(format.block))
        {
            return hr::DSERR_INVALIDPARAM;
        }
        if !capture_present() {
            return hr::DSERR_NODRIVER;
        }
        let Some((section, address, mapped)) = create_ring() else {
            return hr::DSERR_OUTOFMEMORY;
        };
        let capture = match audio(|a| a.capture_open(section, &executable_name())) {
            Some(Ok(id)) => id,
            failed => {
                release_ring(section, address, mapped);
                return match failed {
                    // Privacy & security > Microphone refuses desktop apps.
                    Some(Err(door_service_client::Error::Service(
                        door_service_protocol::Code::Denied,
                    ))) => E_ACCESSDENIED,
                    Some(_) => hr::DSERR_ALLOCATED,
                    None => hr::DSERR_NODRIVER,
                };
            }
        };
        let id = next_id();
        let buffer = Buffer {
            id,
            device,
            refs: 1,
            iface: new_iface(&CAPTURE_BUFFER as *const _ as u64, id),
            notify_iface: new_iface(&CAPTURE_NOTIFY as *const _ as u64, id),
            flags,
            format,
            data: vec![if format.bits == 8 { 0x80 } else { 0 }; bytes as usize],
            cursor: 0,
            capturing: false,
            looping: false,
            notify: Vec::new(),
            capture,
            section,
            ring: address,
            mapped,
            recorder: Recorder::new(wave),
            converted: Vec::new(),
        };
        let iface = buffer.iface;
        CAPTURE.with(|o| o.buffers.push(buffer));
        start_worker();
        result(write_pointer(out, iface))
    }
}
/// DSCCAPS: dwSize (16), dwFlags, dwFormats (every WAVE_FORMAT_* rate and
/// size, as Windows reports a microphone), dwChannels.
extern "system" fn c_get_caps(this: usize, out: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let out = door_windows_abi::widen(out);
    {
        if let Err(e) = with_capture(this, |_| Ok(())) {
            return e;
        }
        let mut size = [0u8; 4];
        if read(out, &mut size).is_err() || u32::from_le_bytes(size) != 16 {
            return hr::DSERR_INVALIDPARAM;
        }
        let caps: Vec<u8> = [16u32, 0, 0x000F_FFFF, 2]
            .iter()
            .flat_map(|v| v.to_le_bytes())
            .collect();
        result(write(out, &caps))
    }
}
extern "system" fn c_initialize(this: usize, guid: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let guid = door_windows_abi::widen(guid);
    {
        if with_capture_object(this, |d| Ok(d.initialized)) == Ok(true) {
            return hr::DSERR_ALREADYINITIALIZED;
        }
        match is_capture_device(guid) {
            Ok(true) if capture_present() => (),
            Ok(_) => return hr::DSERR_NODRIVER,
            Err(e) => return e,
        }
        result(with_capture_object(this, |d| {
            d.initialized = true;
            Ok(())
        }))
    }
}

// ---- IDirectSoundCaptureBuffer8 ------------------------------------------------------
#[repr(C)]
struct BufferVtbl {
    query_interface: Fn3,
    add_ref: Fn1,
    release: Fn1,
    get_caps: Fn2,
    get_current_position: Fn3,
    get_format: Fn4,
    get_status: Fn2,
    initialize: Fn3,
    lock: extern "system" fn(usize, u32, u32, usize, usize, usize, usize, u32) -> u32,
    start: Fn2u,
    stop: Fn1,
    unlock: extern "system" fn(usize, usize, u32, usize, u32) -> u32,
    get_object_in_path: extern "system" fn(usize, usize, u32, usize, usize) -> u32,
    get_fx_status: Fn3,
}
static CAPTURE_BUFFER: BufferVtbl = BufferVtbl {
    query_interface: b_query_interface,
    add_ref: b_add_ref,
    release: b_release,
    get_caps: b_get_caps,
    get_current_position: b_get_current_position,
    get_format: b_get_format,
    get_status: b_get_status,
    initialize: b_initialize,
    lock: b_lock,
    start: b_start,
    stop: b_stop,
    unlock: b_unlock,
    get_object_in_path: b_get_object_in_path,
    get_fx_status: b_get_fx_status,
};
fn with_buffer<R>(this: u64, f: impl FnOnce(&mut Buffer) -> Result<R, u32>) -> Result<R, u32> {
    let id = iface_id(this).ok_or(hr::E_POINTER)?;
    CAPTURE.with(|o| {
        o.buffers
            .iter_mut()
            .find(|b| b.id == id)
            .map_or(Err(hr::DSERR_INVALIDCALL), f)
    })
}
extern "system" fn b_query_interface(this: usize, iid: usize, out: usize) -> u32 {
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
        let found = with_buffer(this, |b| {
            Ok(
                if [
                    IID_IUNKNOWN,
                    IID_IDIRECTSOUNDCAPTUREBUFFER,
                    IID_IDIRECTSOUNDCAPTUREBUFFER8,
                ]
                .contains(&iid)
                {
                    Some(b.iface)
                } else if iid == IID_IDIRECTSOUNDNOTIFY {
                    Some(b.notify_iface)
                } else {
                    None
                },
            )
        });
        match found {
            Ok(Some(iface)) => {
                b_add_ref(door_windows_abi::narrow(this));
                result(write_pointer(out, iface))
            }
            _ => {
                let _ = write_pointer(out, 0);
                hr::DSERR_NOINTERFACE
            }
        }
    }
}
extern "system" fn b_add_ref(this: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    {
        with_buffer(this, |b| {
            b.refs += 1;
            Ok(b.refs)
        })
        .unwrap_or(0)
    }
}
extern "system" fn b_release(this: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    {
        let Some(id) = iface_id(this) else { return 0 };
        let (left, closed) = CAPTURE.with(|o| {
            let Some(at) = o.buffers.iter().position(|b| b.id == id) else {
                return (0, None);
            };
            o.buffers[at].refs -= 1;
            let left = o.buffers[at].refs;
            if left != 0 {
                return (left, None);
            }
            let b = o.buffers.remove(at);
            (0, Some((b.capture, b.section, b.ring, b.mapped)))
        });
        if let Some((capture, section, address, mapped)) = closed {
            audio(|a| a.capture_close(capture));
            release_ring(section, address, mapped);
        }
        left
    }
}
/// DSCBCAPS: dwSize (16), dwFlags, dwBufferBytes, dwReserved.
extern "system" fn b_get_caps(this: usize, out: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let out = door_windows_abi::widen(out);
    {
        let mut size = [0u8; 4];
        if read(out, &mut size).is_err() || u32::from_le_bytes(size) != 16 {
            return hr::DSERR_INVALIDPARAM;
        }
        match with_buffer(this, |b| Ok([16u32, b.flags, b.data.len() as u32, 0])) {
            Ok(c) => result(write(
                out,
                &c.iter().flat_map(|v| v.to_le_bytes()).collect::<Vec<u8>>(),
            )),
            Err(e) => e,
        }
    }
}
/// The capture and read cursors are the same: the worker writes whole
/// converted frames, so everything before the cursor is ready.
extern "system" fn b_get_current_position(this: usize, capture: usize, read_at: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let capture = door_windows_abi::widen(capture);
    let read_at = door_windows_abi::widen(read_at);
    {
        match with_buffer(this, |b| Ok(b.cursor as u32)) {
            Ok(at) => {
                if capture != 0 && write_u32(capture, at).is_err() {
                    return hr::DSERR_INVALIDPARAM;
                }
                if read_at != 0 && write_u32(read_at, at).is_err() {
                    return hr::DSERR_INVALIDPARAM;
                }
                hr::DS_OK
            }
            Err(e) => e,
        }
    }
}
extern "system" fn b_get_format(this: usize, out: usize, allocated: usize, written: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let out = door_windows_abi::widen(out);
    let allocated = door_windows_abi::widen(allocated);
    let written = door_windows_abi::widen(written);
    {
        let Ok(format) = with_buffer(this, |b| Ok(b.format)) else {
            return hr::DSERR_INVALIDCALL;
        };
        let bytes = format.encode();
        if written != 0 && write_u32(written, bytes.len() as u32).is_err() {
            return hr::DSERR_INVALIDPARAM;
        }
        if out == 0 {
            return if written == 0 {
                hr::DSERR_INVALIDPARAM
            } else {
                hr::DS_OK
            };
        }
        if (allocated as u32 as usize) < bytes.len() {
            return hr::DSERR_INVALIDPARAM;
        }
        result(write(out, &bytes))
    }
}
extern "system" fn b_get_status(this: usize, out: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let out = door_windows_abi::widen(out);
    {
        let status = with_buffer(this, |b| {
            Ok(if b.capturing {
                DSCBSTATUS_CAPTURING | if b.looping { DSCBSTATUS_LOOPING } else { 0 }
            } else {
                0
            })
        });
        match status {
            Ok(s) => result(write_u32(out, s)),
            Err(e) => e,
        }
    }
}
extern "system" fn b_initialize(_this: usize, _device: usize, _desc: usize) -> u32 {
    hr::DSERR_ALREADYINITIALIZED
}
extern "system" fn b_lock(
    this: usize,
    offset: u32,
    bytes: u32,
    p1: usize,
    n1: usize,
    p2: usize,
    n2: usize,
    flags: u32,
) -> u32 {
    let this = door_windows_abi::widen(this);
    let p1 = door_windows_abi::widen(p1);
    let n1 = door_windows_abi::widen(n1);
    let p2 = door_windows_abi::widen(p2);
    let n2 = door_windows_abi::widen(n2);
    {
        if p1 == 0 || n1 == 0 {
            return hr::DSERR_INVALIDPARAM;
        }
        let regions = with_buffer(this, |b| {
            let size = b.data.len();
            let bytes = if flags & DSCBLOCK_ENTIREBUFFER != 0 {
                size
            } else {
                bytes as usize
            };
            let ((o1, l1), (o2, l2)) =
                lock_regions(offset as usize, bytes, size).ok_or(hr::DSERR_INVALIDPARAM)?;
            let base = b.data.as_ptr() as u64;
            Ok((
                base + o1 as u64,
                l1,
                if l2 == 0 { 0 } else { base + o2 as u64 },
                l2,
            ))
        });
        let (a1, l1, a2, l2) = match regions {
            Ok(r) => r,
            Err(e) => return e,
        };
        if l2 != 0 && p2 == 0 {
            return hr::DSERR_INVALIDPARAM;
        }
        let mut r = write_pointer(p1, a1).and_then(|_| write_u32(n1, l1 as u32));
        if p2 != 0 {
            r = r.and_then(|_| write_pointer(p2, a2));
        }
        if n2 != 0 {
            r = r.and_then(|_| write_u32(n2, l2 as u32));
        }
        result(r)
    }
}
extern "system" fn b_start(this: usize, flags: u32) -> u32 {
    let this = door_windows_abi::widen(this);
    {
        result(with_buffer(this, |b| {
            if !b.capturing {
                // Sound captured before the start is not recorded.
                let written = ring_count(b.ring, ring::WRITE).load(Ordering::Acquire);
                ring_count(b.ring, ring::READ).store(written, Ordering::Release);
                b.converted.clear();
            }
            b.looping = flags & DSCBSTART_LOOPING != 0;
            b.capturing = true;
            Ok(())
        }))
    }
}
extern "system" fn b_stop(this: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    {
        let events = with_buffer(this, |b| {
            let was = b.capturing;
            b.capturing = false;
            b.converted.clear();
            Ok(if was {
                b.notify
                    .iter()
                    .filter(|(o, _)| *o == DSBPN_OFFSETSTOP)
                    .map(|(_, e)| *e)
                    .collect()
            } else {
                Vec::new()
            })
        });
        match events {
            Ok(events) => {
                for e in events {
                    unsafe { DoorSetEvent(e) };
                }
                hr::DS_OK
            }
            Err(e) => e,
        }
    }
}
extern "system" fn b_unlock(this: usize, _p1: usize, _n1: u32, _p2: usize, _n2: u32) -> u32 {
    let this = door_windows_abi::widen(this);
    result(with_buffer(this, |_| Ok(())))
}
extern "system" fn b_get_object_in_path(
    _this: usize,
    _guid: usize,
    _index: u32,
    _iid: usize,
    out: usize,
) -> u32 {
    let out = door_windows_abi::widen(out);
    {
        let _ = write_pointer(out, 0);
        hr::DSERR_CONTROLUNAVAIL
    }
}
extern "system" fn b_get_fx_status(this: usize, _count: usize, _results: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    {
        match with_buffer(this, |_| Ok(())) {
            Ok(()) => hr::DSERR_CONTROLUNAVAIL,
            Err(e) => e,
        }
    }
}

// ---- IDirectSoundNotify on capture buffers --------------------------------------------
#[repr(C)]
struct NotifyVtbl {
    query_interface: Fn3,
    add_ref: Fn1,
    release: Fn1,
    set_notification_positions: extern "system" fn(usize, u32, usize) -> u32,
}
static CAPTURE_NOTIFY: NotifyVtbl = NotifyVtbl {
    query_interface: b_query_interface,
    add_ref: b_add_ref,
    release: b_release,
    set_notification_positions: n_set_notification_positions,
};
extern "system" fn n_set_notification_positions(this: usize, count: u32, positions: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let positions = door_windows_abi::widen(positions);
    {
        let list = match notification_list(count, positions) {
            Ok(l) => l,
            Err(e) => return e,
        };
        result(with_buffer(this, |b| {
            if b.capturing {
                return Err(hr::DSERR_INVALIDCALL);
            }
            if list
                .iter()
                .any(|(o, _)| *o != DSBPN_OFFSETSTOP && *o as usize >= b.data.len())
            {
                return Err(hr::DSERR_INVALIDPARAM);
            }
            b.notify = list;
            Ok(())
        }))
    }
}

// ---- IDirectSoundFullDuplex ---------------------------------------------------------
/// The object DirectSoundFullDuplexCreate makes: it holds one reference on
/// the playback and the capture device it opened.
struct Duplex {
    id: u32,
    refs: u32,
    render: u64,
    capture: u64,
}
static DUPLEX: Lock<Vec<Duplex>> = Lock::new(Vec::new());
#[repr(C)]
struct DuplexVtbl {
    query_interface: Fn3,
    add_ref: Fn1,
    release: Fn1,
    initialize:
        extern "system" fn(usize, usize, usize, usize, usize, usize, u32, usize, usize) -> u32,
}
static DUPLEX_VTBL: DuplexVtbl = DuplexVtbl {
    query_interface: f_query_interface,
    add_ref: f_add_ref,
    release: f_release,
    initialize: f_initialize,
};
pub(super) extern "system" fn f_query_interface(this: usize, iid: usize, out: usize) -> u32 {
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
        let Some(id) = iface_id(this) else {
            return hr::E_POINTER;
        };
        let Some((render, capture)) =
            DUPLEX.with(|d| d.iter().find(|x| x.id == id).map(|x| (x.render, x.capture)))
        else {
            return hr::DSERR_INVALIDCALL;
        };
        // The full-duplex object answers for its two devices too.
        let target = if [IID_IUNKNOWN, IID_IDIRECTSOUNDFULLDUPLEX].contains(&iid) {
            f_add_ref(door_windows_abi::narrow(this));
            this
        } else if [IID_IDIRECTSOUND, IID_IDIRECTSOUND8].contains(&iid) && render != 0 {
            ds_add_ref(door_windows_abi::narrow(render));
            render
        } else if iid == IID_IDIRECTSOUNDCAPTURE && capture != 0 {
            c_add_ref(door_windows_abi::narrow(capture));
            capture
        } else {
            let _ = write_pointer(out, 0);
            return hr::DSERR_NOINTERFACE;
        };
        result(write_pointer(out, target))
    }
}
extern "system" fn f_add_ref(this: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    {
        let Some(id) = iface_id(this) else { return 0 };
        DUPLEX.with(|d| {
            d.iter_mut().find(|x| x.id == id).map_or(0, |x| {
                x.refs += 1;
                x.refs
            })
        })
    }
}
pub(super) extern "system" fn f_release(this: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    {
        let Some(id) = iface_id(this) else { return 0 };
        let (left, devices) = DUPLEX.with(|d| {
            let Some(at) = d.iter().position(|x| x.id == id) else {
                return (0, None);
            };
            d[at].refs -= 1;
            if d[at].refs != 0 {
                return (d[at].refs, None);
            }
            let x = d.remove(at);
            (0, Some((x.render, x.capture)))
        });
        if let Some((render, capture)) = devices
            && render != 0
        {
            ds_release(door_windows_abi::narrow(render));
            c_release(door_windows_abi::narrow(capture));
        }
        left
    }
}
/// Opens the two devices and their buffers (DirectSoundFullDuplexCreate,
/// or a full-duplex object CoCreateInstance made).
#[allow(clippy::too_many_arguments)]
extern "system" fn f_initialize(
    this: usize,
    capture_guid: usize,
    render_guid: usize,
    capture_desc: usize,
    render_desc: usize,
    window: usize,
    level: u32,
    capture_out: usize,
    render_out: usize,
) -> u32 {
    let this = door_windows_abi::widen(this);
    let capture_guid = door_windows_abi::widen(capture_guid);
    let render_guid = door_windows_abi::widen(render_guid);
    let capture_desc = door_windows_abi::widen(capture_desc);
    let render_desc = door_windows_abi::widen(render_desc);
    let window = door_windows_abi::widen(window);
    let capture_out = door_windows_abi::widen(capture_out);
    let render_out = door_windows_abi::widen(render_out);
    {
        if capture_out == 0 || render_out == 0 {
            return hr::DSERR_INVALIDPARAM;
        }
        for p in [capture_out, render_out] {
            let _ = write_pointer(p, 0);
        }
        let Some(id) = iface_id(this) else {
            return hr::E_POINTER;
        };
        match DUPLEX.with(|d| d.iter().find(|x| x.id == id).map(|x| x.render != 0)) {
            None => return hr::DSERR_INVALIDCALL,
            Some(true) => return hr::DSERR_ALREADYINITIALIZED,
            Some(false) => (),
        }
        let mut render = [0u8; 8];
        let r = super::DirectSoundCreate8(
            door_windows_abi::narrow(render_guid),
            door_windows_abi::narrow(render.as_mut_ptr() as u64),
            0,
        );
        if r != hr::DS_OK {
            return r;
        }
        let render = u64::from_le_bytes(render);
        let mut capture = [0u8; 8];
        let c = create(capture_guid, capture.as_mut_ptr() as u64, 0);
        if c != hr::DS_OK {
            ds_release(door_windows_abi::narrow(render));
            return c;
        }
        let capture = u64::from_le_bytes(capture);
        let fail = |e: u32| {
            c_release(door_windows_abi::narrow(capture));
            ds_release(door_windows_abi::narrow(render));
            e
        };
        let level = ds_set_cooperative_level(
            door_windows_abi::narrow(render),
            door_windows_abi::narrow(window),
            door_windows_abi::narrow(u64::from(level)),
        );
        if level != hr::DS_OK {
            return fail(level);
        }
        let mut cb = [0u8; 8];
        let made = c_create_capture_buffer(
            door_windows_abi::narrow(capture),
            door_windows_abi::narrow(capture_desc),
            door_windows_abi::narrow(cb.as_mut_ptr() as u64),
            0,
        );
        if made != hr::DS_OK {
            return fail(made);
        }
        let mut rb = [0u8; 8];
        let made = ds_create_sound_buffer(
            door_windows_abi::narrow(render),
            door_windows_abi::narrow(render_desc),
            door_windows_abi::narrow(rb.as_mut_ptr() as u64),
            0,
        );
        if made != hr::DS_OK {
            b_release(door_windows_abi::narrow(u64::from_le_bytes(cb)));
            return fail(made);
        }
        DUPLEX.with(|d| {
            if let Some(x) = d.iter_mut().find(|x| x.id == id) {
                x.render = render;
                x.capture = capture;
            }
        });
        result(
            write_pointer(capture_out, u64::from_le_bytes(cb))
                .and_then(|_| write_pointer(render_out, u64::from_le_bytes(rb))),
        )
    }
}
/// A full-duplex object waiting for Initialize.
pub(super) fn new_duplex() -> u64 {
    let id = next_id();
    let iface = new_iface(&DUPLEX_VTBL as *const _ as u64, id);
    DUPLEX.with(|d| {
        d.push(Duplex {
            id,
            refs: 1,
            render: 0,
            capture: 0,
        })
    });
    iface
}

// ---- exports ------------------------------------------------------------------
fn is_capture_device(guid: u64) -> Result<bool, u32> {
    if guid == 0 {
        return Ok(true);
    }
    let g = read_guid(guid).map_err(|_| hr::DSERR_INVALIDPARAM)?;
    Ok([
        DSDEVID_DEFAULT_CAPTURE,
        DSDEVID_DEFAULT_VOICE_CAPTURE,
        MICROPHONE,
    ]
    .contains(&g))
}
pub(super) fn is_default_capture(g: &Guid) -> bool {
    [DSDEVID_DEFAULT_CAPTURE, DSDEVID_DEFAULT_VOICE_CAPTURE].contains(g)
}
fn create(guid: u64, out: u64, outer: u64) -> u32 {
    if out == 0 {
        return hr::DSERR_INVALIDPARAM;
    }
    let _ = write_pointer(out, 0);
    if outer != 0 {
        return hr::DSERR_NOAGGREGATION;
    }
    match is_capture_device(guid) {
        Ok(true) => (),
        Ok(false) => return hr::DSERR_NODRIVER,
        Err(e) => return e,
    }
    if !capture_present() {
        return hr::DSERR_NODRIVER;
    }
    result(write_pointer(out, new_capture(true)))
}
/// A capture device object, initialized or waiting for Initialize.
pub(super) fn new_capture(initialized: bool) -> u64 {
    let id = next_id();
    let iface = new_iface(&CAPTURE_VTBL as *const _ as u64, id);
    CAPTURE.with(|o| {
        o.devices.push(Device {
            id,
            refs: 1,
            initialized,
        })
    });
    iface
}
#[unsafe(no_mangle)]
pub extern "system" fn DirectSoundCaptureCreate(guid: usize, out: usize, outer: usize) -> u32 {
    let guid = door_windows_abi::widen(guid);
    let out = door_windows_abi::widen(out);
    let outer = door_windows_abi::widen(outer);
    create(guid, out, outer)
}
#[unsafe(no_mangle)]
pub extern "system" fn DirectSoundCaptureCreate8(guid: usize, out: usize, outer: usize) -> u32 {
    let guid = door_windows_abi::widen(guid);
    let out = door_windows_abi::widen(out);
    let outer = door_windows_abi::widen(outer);
    create(guid, out, outer)
}
fn enumerate(callback: u64, context: u64, wide: bool) -> u32 {
    let devices: &[(Option<&Guid>, &str, &str)] = &[
        (None, "Primary Sound Capture Driver", ""),
        (
            Some(&MICROPHONE),
            MICROPHONE_NAME,
            "{0.0.1.00000000}.{d0025d01-5ea4-4a6b-9c11-2b6e15500002}",
        ),
    ];
    let shown = if capture_present() {
        devices
    } else {
        &devices[..0]
    };
    enumerate_devices(callback, context, wide, shown)
}
#[unsafe(no_mangle)]
pub extern "system" fn DirectSoundCaptureEnumerateW(callback: usize, context: usize) -> u32 {
    let callback = door_windows_abi::widen(callback);
    let context = door_windows_abi::widen(context);
    enumerate(callback, context, true)
}
#[unsafe(no_mangle)]
pub extern "system" fn DirectSoundCaptureEnumerateA(callback: usize, context: usize) -> u32 {
    let callback = door_windows_abi::widen(callback);
    let context = door_windows_abi::widen(context);
    enumerate(callback, context, false)
}
/// Opens a playback and a capture device with one buffer each.
#[allow(clippy::too_many_arguments)]
#[unsafe(no_mangle)]
pub extern "system" fn DirectSoundFullDuplexCreate(
    capture_guid: usize,
    render_guid: usize,
    capture_desc: usize,
    render_desc: usize,
    window: usize,
    level: u32,
    out: usize,
    capture_out: usize,
    render_out: usize,
    outer: usize,
) -> u32 {
    let capture_guid = door_windows_abi::widen(capture_guid);
    let render_guid = door_windows_abi::widen(render_guid);
    let capture_desc = door_windows_abi::widen(capture_desc);
    let render_desc = door_windows_abi::widen(render_desc);
    let window = door_windows_abi::widen(window);
    let out = door_windows_abi::widen(out);
    let capture_out = door_windows_abi::widen(capture_out);
    let render_out = door_windows_abi::widen(render_out);
    let outer = door_windows_abi::widen(outer);
    {
        if out == 0 {
            return hr::DSERR_INVALIDPARAM;
        }
        let _ = write_pointer(out, 0);
        if outer != 0 {
            return hr::DSERR_NOAGGREGATION;
        }
        let duplex = new_duplex();
        let r = f_initialize(
            door_windows_abi::narrow(duplex),
            door_windows_abi::narrow(capture_guid),
            door_windows_abi::narrow(render_guid),
            door_windows_abi::narrow(capture_desc),
            door_windows_abi::narrow(render_desc),
            door_windows_abi::narrow(window),
            level,
            door_windows_abi::narrow(capture_out),
            door_windows_abi::narrow(render_out),
        );
        if r != hr::DS_OK {
            f_release(door_windows_abi::narrow(duplex));
            return r;
        }
        result(write_pointer(out, duplex))
    }
}
/// Any capture or full-duplex object alive (DllCanUnloadNow).
pub(super) fn live() -> bool {
    CAPTURE.with(|o| !o.devices.is_empty() || !o.buffers.is_empty())
        || DUPLEX.with(|d| !d.is_empty())
}
