//! dsound.dll exports and its COM objects. One device object per
//! DirectSoundCreate(8); its secondary buffers are mixed by one worker
//! thread into a door.audio stream (48 kHz, 16-bit stereo), 100 ms per
//! section with two queued. Every 10 ms of a mixed section records where
//! each playing buffer stood, so the play cursor is what
//! door.audio has played and the write cursor is what has been mixed.
//! Position notifications set their events as the play cursor passes them.
//! Buffer memory is this DLL's own heap: Lock hands the application
//! pointers into it, and the mixer reads it in place.
use crate::*;
use alloc::{boxed::Box, collections::VecDeque, vec, vec::Vec};
use core::sync::atomic::{AtomicBool, Ordering};

mod capture;
mod factory;
use door_service_client::audio::Audio;
use door_service_protocol::audio as proto;

struct Lock<T> {
    held: AtomicBool,
    value: core::cell::UnsafeCell<T>,
}
unsafe impl<T: Send> Sync for Lock<T> {}
impl<T> Lock<T> {
    const fn new(value: T) -> Self {
        Self {
            held: AtomicBool::new(false),
            value: core::cell::UnsafeCell::new(value),
        }
    }
    fn with<R>(&self, f: impl FnOnce(&mut T) -> R) -> R {
        while self.held.swap(true, Ordering::Acquire) {
            core::hint::spin_loop();
        }
        // SAFETY: `held` gives this closure exclusive access.
        let r = f(unsafe { &mut *self.value.get() });
        self.held.store(false, Ordering::Release);
        r
    }
}

#[global_allocator]
static HEAP: door_windows_abi::allocator::ProcessHeap = door_windows_abi::allocator::ProcessHeap;

#[link(name = "ntdll", kind = "dylib")]
unsafe extern "system" {
    fn DoorUserCopy(destination: u64, source: u64, bytes: usize) -> u32;
    fn DoorGetInheritedHandle(index: u32) -> u64;
    fn DoorReserveSectionRange(bytes: u64) -> u64;
    fn DoorReleaseSectionRange(address: u64, bytes: u64) -> i32;
    fn DoorCreateThreadEx(entry: u64, argument: u64, id: u64) -> u64;
    fn DoorCloseHandle(handle: u64) -> i32;
    fn DoorSleep(millis: u32);
    fn DoorSetEvent(handle: u64) -> i32;
}

// ---- application memory --------------------------------------------------
const COPY_CHUNK: usize = door_windows_abi::APPLICATION_COPY_BYTES;
fn read(pointer: u64, out: &mut [u8]) -> Result<(), u32> {
    if pointer == 0 {
        return Err(hr::DSERR_INVALIDPARAM);
    }
    for (i, chunk) in out.chunks_mut(COPY_CHUNK).enumerate() {
        if unsafe {
            DoorUserCopy(
                chunk.as_mut_ptr() as u64,
                pointer + (i * COPY_CHUNK) as u64,
                chunk.len(),
            )
        } != 0
        {
            return Err(hr::DSERR_INVALIDPARAM);
        }
    }
    Ok(())
}
fn write(pointer: u64, data: &[u8]) -> Result<(), u32> {
    if pointer == 0 {
        return Err(hr::DSERR_INVALIDPARAM);
    }
    for (i, chunk) in data.chunks(COPY_CHUNK).enumerate() {
        if unsafe {
            DoorUserCopy(
                pointer + (i * COPY_CHUNK) as u64,
                chunk.as_ptr() as u64,
                chunk.len(),
            )
        } != 0
        {
            return Err(hr::DSERR_INVALIDPARAM);
        }
    }
    Ok(())
}
fn write_u32(pointer: u64, value: u32) -> Result<(), u32> {
    write(pointer, &value.to_le_bytes())
}
/// The program's pointer width (P17: 4 for a 32-bit program).
pub(crate) const POINTER: usize = door_windows_abi::layout::POINTER;
/// An interface or buffer pointer into an out-parameter (P17: at the
/// program's width).
fn write_pointer(pointer: u64, value: u64) -> Result<(), u32> {
    write(pointer, &value.to_le_bytes()[..POINTER])
}
/// DSBUFFERDESC1 and DSBUFFERDESC (dwSize, dwFlags, dwBufferBytes,
/// dwReserved, lpwfxFormat[, guid3DAlgorithm]) and their capture
/// counterparts DSCBUFFERDESC1 and DSCBUFFERDESC ([dwFXCount,
/// lpDSCFXDesc]): the format pointer at 16 in each, the sizes by width.
pub(crate) const DESC1: u32 = (16 + POINTER) as u32;
pub(crate) const DESC: u32 = door_windows_abi::layout::size(&{
    use door_windows_abi::layout::Field::*;
    [U32, U32, U32, U32, Pointer, Bytes(16)]
}) as u32;
pub(crate) const CAPTURE_DESC: u32 = door_windows_abi::layout::size(&{
    use door_windows_abi::layout::Field::*;
    [U32, U32, U32, U32, Pointer, U32, Pointer]
}) as u32;
/// The lpwfxFormat of a buffer description read into `d`.
pub(crate) fn format_pointer(d: &[u8]) -> u64 {
    door_windows_abi::layout::get(&d[16..], &[door_windows_abi::layout::Field::Pointer], 0)
}
fn result(r: Result<(), u32>) -> u32 {
    r.err().unwrap_or(hr::DS_OK)
}

// ---- GUIDs -------------------------------------------------------------
type Guid = [u8; 16];
const fn guid(a: u32, b: u16, c: u16, d: [u8; 8]) -> Guid {
    let a = a.to_le_bytes();
    let b = b.to_le_bytes();
    let c = c.to_le_bytes();
    [
        a[0], a[1], a[2], a[3], b[0], b[1], c[0], c[1], d[0], d[1], d[2], d[3], d[4], d[5], d[6],
        d[7],
    ]
}
const IID_IUNKNOWN: Guid = guid(0, 0, 0, [0xC0, 0, 0, 0, 0, 0, 0, 0x46]);
const IID_IDIRECTSOUND: Guid = guid(
    0x279A_FA83,
    0x4981,
    0x11CE,
    [0xA5, 0x21, 0x00, 0x20, 0xAF, 0x0B, 0xE5, 0x60],
);
const IID_IDIRECTSOUND8: Guid = guid(
    0xC50A_7E93,
    0xF395,
    0x4834,
    [0x9E, 0xF6, 0x7F, 0xA9, 0x9D, 0xE5, 0x09, 0x66],
);
const IID_IDIRECTSOUNDBUFFER: Guid = guid(
    0x279A_FA85,
    0x4981,
    0x11CE,
    [0xA5, 0x21, 0x00, 0x20, 0xAF, 0x0B, 0xE5, 0x60],
);
const IID_IDIRECTSOUNDBUFFER8: Guid = guid(
    0x6825_A449,
    0x7524,
    0x4D82,
    [0x92, 0x0F, 0x50, 0xE3, 0x6A, 0xB3, 0xAB, 0x1E],
);
const IID_IDIRECTSOUNDNOTIFY: Guid = guid(
    0xB021_0783,
    0x89CD,
    0x11D0,
    [0xAF, 0x08, 0x00, 0xA0, 0xC9, 0x25, 0xCD, 0x16],
);
const DSDEVID_DEFAULT_PLAYBACK: Guid = guid(
    0xDEF0_0000,
    0x9C6D,
    0x47ED,
    [0xAA, 0xF1, 0x4D, 0xDA, 0x8F, 0x2B, 0x5C, 0x03],
);
const DSDEVID_DEFAULT_VOICE_PLAYBACK: Guid = guid(
    0xDEF0_0002,
    0x9C6D,
    0x47ED,
    [0xAA, 0xF1, 0x4D, 0xDA, 0x8F, 0x2B, 0x5C, 0x03],
);
/// The Doors speaker's device GUID (stable, its own).
const SPEAKER: Guid = guid(
    0xD002_5D01,
    0x5EA4,
    0x4A6B,
    [0x9C, 0x11, 0x2B, 0x6E, 0x15, 0x50, 0x00, 0x01],
);
const SPEAKER_NAME: &str = "Speakers (VirtIO Sound)";
fn read_guid(pointer: u64) -> Result<Guid, u32> {
    let mut g = [0u8; 16];
    read(pointer, &mut g)?;
    Ok(g)
}

// ---- door.audio ----------------------------------------------------------
static AUDIO: Lock<Option<Audio>> = Lock::new(None);
fn audio<R>(f: impl FnOnce(&mut Audio) -> R) -> Option<R> {
    AUDIO.with(|a| {
        if a.is_none() {
            let registry =
                unsafe { DoorGetInheritedHandle(door_abi::platform::inherited::REGISTRY as u32) };
            *a = Audio::connect(registry).ok();
        }
        a.as_mut().map(f)
    })
}
fn device_present() -> bool {
    audio(|a| a.status().is_ok_and(|s| s.present)).unwrap_or(false)
}
/// A section holding `data` (winmm's pattern: the reserved range goes back
/// only once nothing is mapped in it).
fn section(data: &[u8]) -> Option<u64> {
    let mapped = (data.len() as u64).div_ceil(4096).max(1) * 4096;
    let address = unsafe { DoorReserveSectionRange(mapped) };
    if address == 0 {
        return None;
    }
    let release = || unsafe {
        DoorReleaseSectionRange(address, mapped);
    };
    let Ok(handle) =
        door_sys::section_create(mapped, door_abi::rights::ALL & !door_abi::rights::WAIT)
    else {
        release();
        return None;
    };
    // SAFETY: the reserved range is this section's alone; the copy ends
    // before the unmap and before the handle leaves this process.
    let filled = unsafe {
        door_sys::section_map(
            handle,
            address,
            door_abi::rights::READ | door_abi::rights::WRITE,
        )
    }
    .map(|_| unsafe {
        core::ptr::copy_nonoverlapping(data.as_ptr(), address as *mut u8, data.len())
    });
    if filled.is_err() || unsafe { door_sys::section_unmap(address, mapped) }.is_ok() {
        release();
    }
    if filled.is_err() {
        let _ = door_sys::close(handle);
        return None;
    }
    Some(handle)
}

// ---- objects ---------------------------------------------------------------
/// The COM-visible part of every object: its table and our key.
#[repr(C)]
struct Iface {
    vtbl: u64,
    id: u32,
}
fn new_iface(vtbl: u64, id: u32) -> u64 {
    Box::into_raw(Box::new(Iface { vtbl, id })) as u64
}
fn iface_id(this: u64) -> Option<u32> {
    // SAFETY: `this` is an interface pointer this DLL handed out.
    (this != 0).then(|| unsafe { (*(this as *const Iface)).id })
}
struct Device {
    id: u32,
    refs: u32,
    /// Made by DirectSoundCreate(8), or by CoCreateInstance and then
    /// Initialize: until then every method but Initialize refuses.
    initialized: bool,
    cooperative: u32,
    primary_format: Format,
    stream: Option<u32>,
    /// Frames handed to door.audio, and the end of each queued section.
    mixed: u64,
    queued: VecDeque<u64>,
    /// The primary buffer was played: keep the stream running.
    running: bool,
    /// A mixed section door.audio refused (it goes first next time).
    pending: Option<Vec<u8>>,
}
struct Buffer {
    id: u32,
    device: u32,
    refs: u32,
    iface: u64,
    notify_iface: u64,
    flags: u32,
    primary: bool,
    data: Vec<u8>,
    voice: Voice,
    volume: i32,
    pan: i32,
    frequency: u32,
    notify: Vec<(u32, u64)>,
    /// (mixed frame where a section ended, byte position there), oldest
    /// first, while door.audio has not played them.
    marks: VecDeque<(u64, usize)>,
    play: usize,
    /// Mixed frame where a one-shot voice ran out (it stops sounding there).
    ends_at: Option<u64>,
}
struct Objects {
    devices: Vec<Device>,
    buffers: Vec<Buffer>,
    next: u32,
}
static OBJECTS: Lock<Objects> = Lock::new(Objects {
    devices: Vec::new(),
    buffers: Vec::new(),
    next: 1,
});
static WORKER: AtomicBool = AtomicBool::new(false);

fn with_object<R>(this: u64, f: impl FnOnce(&mut Device) -> Result<R, u32>) -> Result<R, u32> {
    let id = iface_id(this).ok_or(hr::E_POINTER)?;
    OBJECTS.with(|o| {
        o.devices
            .iter_mut()
            .find(|d| d.id == id)
            .map_or(Err(hr::DSERR_INVALIDCALL), f)
    })
}
/// An initialized device.
fn with_device<R>(this: u64, f: impl FnOnce(&mut Device) -> Result<R, u32>) -> Result<R, u32> {
    with_object(this, |d| {
        if d.initialized {
            f(d)
        } else {
            Err(hr::DSERR_UNINITIALIZED)
        }
    })
}
fn with_buffer<R>(
    this: u64,
    f: impl FnOnce(&mut Buffer, &mut Option<&mut Device>) -> Result<R, u32>,
) -> Result<R, u32> {
    let id = iface_id(this).ok_or(hr::E_POINTER)?;
    OBJECTS.with(|o| {
        let Some(b) = o.buffers.iter_mut().find(|b| b.id == id) else {
            return Err(hr::DSERR_INVALIDCALL);
        };
        let mut device = o.devices.iter_mut().find(|d| d.id == b.device);
        f(b, &mut device)
    })
}

// ---- the mixer -----------------------------------------------------------
/// Frames per mixed section (100 ms) and sections queued at once: door.audio
/// takes two per stream, and keeps the device ten 10 ms periods ahead, so
/// the write cursor leads the play cursor by at most 200 ms.
const SECTION_FRAMES: usize = MIX_RATE as usize / 10;
const QUEUED: usize = 2;
/// Cursor marks within a section: one per 10 ms period.
const MARK_FRAMES: usize = MIX_RATE as usize / 100;

/// One worker pass: retire what door.audio played (cursors, ends,
/// notifications), then top up each running device's queue. A section
/// door.audio refuses stays pending and goes first next time: the voices
/// already moved past it.
fn pass() -> Vec<u64> {
    let mut events = Vec::new();
    let streams: Vec<(u32, u32)> = OBJECTS.with(|o| {
        o.devices
            .iter()
            .filter_map(|d| d.stream.map(|s| (d.id, s)))
            .collect()
    });
    for (device, stream) in streams {
        let played = audio(|a| a.stream(stream).ok().map(|s| s.played))
            .flatten()
            .unwrap_or(0);
        OBJECTS.with(|o| {
            let Some(d) = o.devices.iter_mut().find(|d| d.id == device) else {
                return;
            };
            while d.queued.front().is_some_and(|end| *end <= played) {
                d.queued.pop_front();
            }
            for b in o
                .buffers
                .iter_mut()
                .filter(|b| b.device == device && !b.primary)
            {
                let before = b.play;
                while let Some(&(end, position)) = b.marks.front() {
                    if end > played {
                        break;
                    }
                    b.marks.pop_front();
                    b.play = position;
                }
                events.extend(crossed(&b.notify, before, b.play, b.data.len()));
                if b.ends_at.is_some_and(|end| end <= played) {
                    b.ends_at = None;
                    events.extend(
                        b.notify
                            .iter()
                            .filter(|(o, _)| *o == DSBPN_OFFSETSTOP)
                            .map(|(_, e)| *e),
                    );
                }
            }
        });
        loop {
            let chunk = OBJECTS.with(|o| {
                let d = o.devices.iter_mut().find(|d| d.id == device)?;
                if d.queued.len() >= QUEUED {
                    return None;
                }
                if let Some(pcm) = d.pending.take() {
                    return Some(pcm);
                }
                let any = o
                    .buffers
                    .iter()
                    .any(|b| b.device == device && !b.primary && b.voice.playing);
                if !any && !d.running {
                    return None;
                }
                let start = d.mixed;
                let mut mix = vec![0i32; SECTION_FRAMES * 2];
                for b in o
                    .buffers
                    .iter_mut()
                    .filter(|b| b.device == device && !b.primary && b.voice.playing)
                {
                    for (k, part) in mix.chunks_mut(MARK_FRAMES * 2).enumerate() {
                        let end = start + ((k + 1) * MARK_FRAMES) as u64;
                        if b.voice.mix(&b.data, part) {
                            b.ends_at = Some(end);
                        }
                        b.marks
                            .push_back((end, b.voice.byte_position() % b.data.len().max(1)));
                        if !b.voice.playing {
                            break;
                        }
                    }
                }
                let mut pcm = vec![0u8; SECTION_FRAMES * 4];
                to_pcm16(&mix, &mut pcm);
                Some(pcm)
            });
            let Some(pcm) = chunk else { break };
            let queued = section(&pcm).is_some_and(|handle| {
                let queued = audio(|a| a.queue(stream, handle, pcm.len()).is_ok()).unwrap_or(false);
                let _ = door_sys::close(handle);
                queued
            });
            OBJECTS.with(|o| {
                if let Some(d) = o.devices.iter_mut().find(|d| d.id == device) {
                    if queued {
                        d.mixed += SECTION_FRAMES as u64;
                        let end = d.mixed;
                        d.queued.push_back(end);
                    } else {
                        d.pending = Some(pcm);
                    }
                }
            });
            if !queued {
                break;
            }
        }
    }
    events
}
extern "system" fn worker(_argument: usize) -> u32 {
    {
        loop {
            for event in pass().into_iter().chain(capture::pass()) {
                unsafe { DoorSetEvent(event) };
            }
            unsafe { DoorSleep(5) };
        }
    }
}
fn start_worker() {
    if !WORKER.swap(true, Ordering::AcqRel) {
        let thread = unsafe { DoorCreateThreadEx(worker as *const () as u64, 0, 0) };
        if thread == 0 {
            WORKER.store(false, Ordering::Release);
        } else {
            unsafe { DoorCloseHandle(thread) };
        }
    }
}
/// The device's door.audio stream, opened on first use.
fn ensure_stream(d: &mut Device) -> Result<(), u32> {
    if d.stream.is_none() {
        let format = proto::Format {
            rate: MIX_RATE,
            channels: 2,
            bits: 16,
        };
        d.stream = Some(
            audio(|a| a.open(format))
                .and_then(Result::ok)
                .ok_or(hr::DSERR_NODRIVER)?,
        );
    }
    start_worker();
    Ok(())
}

// ---- IDirectSound8 ---------------------------------------------------------
type Fn1 = extern "system" fn(usize) -> u32;
type Fn2 = extern "system" fn(usize, usize) -> u32;
type Fn3 = extern "system" fn(usize, usize, usize) -> u32;
type Fn4 = extern "system" fn(usize, usize, usize, usize) -> u32;
type Fn2u = extern "system" fn(usize, u32) -> u32;
type Fn2i = extern "system" fn(usize, i32) -> u32;

#[repr(C)]
struct DirectSoundVtbl {
    query_interface: Fn3,
    add_ref: Fn1,
    release: Fn1,
    create_sound_buffer: Fn4,
    get_caps: Fn2,
    duplicate_sound_buffer: Fn3,
    set_cooperative_level: Fn3,
    compact: Fn1,
    get_speaker_config: Fn2,
    set_speaker_config: Fn2u,
    initialize: Fn2,
    verify_certification: Fn2,
}
static DIRECT_SOUND: DirectSoundVtbl = DirectSoundVtbl {
    query_interface: ds_query_interface,
    add_ref: ds_add_ref,
    release: ds_release,
    create_sound_buffer: ds_create_sound_buffer,
    get_caps: ds_get_caps,
    duplicate_sound_buffer: ds_duplicate_sound_buffer,
    set_cooperative_level: ds_set_cooperative_level,
    compact: ds_compact,
    get_speaker_config: ds_get_speaker_config,
    set_speaker_config: ds_set_speaker_config,
    initialize: ds_initialize,
    verify_certification: ds_verify_certification,
};
extern "system" fn ds_query_interface(this: usize, iid: usize, out: usize) -> u32 {
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
        if [IID_IUNKNOWN, IID_IDIRECTSOUND, IID_IDIRECTSOUND8].contains(&iid) {
            ds_add_ref(door_windows_abi::narrow(this));
            return result(write_pointer(out, this));
        }
        let _ = write_pointer(out, 0);
        hr::DSERR_NOINTERFACE
    }
}
extern "system" fn ds_add_ref(this: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    {
        with_object(this, |d| {
            d.refs += 1;
            Ok(d.refs)
        })
        .unwrap_or(0)
    }
}
extern "system" fn ds_release(this: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    {
        let Some(id) = iface_id(this) else { return 0 };
        let (left, stream) = OBJECTS.with(|o| {
            let Some(at) = o.devices.iter().position(|d| d.id == id) else {
                return (0, None);
            };
            o.devices[at].refs -= 1;
            let left = o.devices[at].refs;
            if left != 0 {
                return (left, None);
            }
            // Releasing the device releases its buffers (as DirectSound does).
            let d = o.devices.remove(at);
            o.buffers.retain(|b| b.device != id);
            (0, d.stream)
        });
        if let Some(stream) = stream {
            audio(|a| a.close(stream));
        }
        left
    }
}
/// DSBUFFERDESC: dwSize (24 or 40), dwFlags, dwBufferBytes, dwReserved,
/// lpwfxFormat.
extern "system" fn ds_create_sound_buffer(
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
        let mut d = [0u8; 24];
        if read(desc, &mut d).is_err() {
            return hr::DSERR_INVALIDPARAM;
        }
        let word = |at: usize| u32::from_le_bytes(d[at..at + 4].try_into().unwrap());
        let (size, flags, bytes, format_ptr) = (word(0), word(4), word(8), format_pointer(&d));
        if (size != DESC1 && size != DESC) || flags & !caps::VALID != 0 {
            return hr::DSERR_INVALIDPARAM;
        }
        // FX and 3D processing are not implemented: refused, never ignored.
        if flags & (caps::CTRLFX | caps::CTRL3D) != 0 {
            return hr::DSERR_CONTROLUNAVAIL;
        }
        let primary = flags & caps::PRIMARYBUFFER != 0;
        let format = if primary {
            if bytes != 0 || format_ptr != 0 {
                return hr::DSERR_INVALIDPARAM;
            }
            None
        } else {
            let mut f = [0u8; 40];
            if read(format_ptr, &mut f[..18]).is_err() {
                return hr::DSERR_INVALIDPARAM;
            }
            if u16::from_le_bytes([f[0], f[1]]) == WAVE_FORMAT_EXTENSIBLE
                && read(format_ptr + 18, &mut f[18..40]).is_err()
            {
                return hr::DSERR_INVALIDPARAM;
            }
            let Some(format) = Format::parse(&f) else {
                return Format::refusal(&f);
            };
            if !(DSBSIZE_MIN..=DSBSIZE_MAX).contains(&bytes)
                || !(bytes as usize).is_multiple_of(usize::from(format.block))
            {
                return hr::DSERR_INVALIDPARAM;
            }
            Some(format)
        };
        let Some(device) = iface_id(this) else {
            return hr::E_POINTER;
        };
        let made = OBJECTS.with(|o| {
            let d = o
                .devices
                .iter()
                .find(|d| d.id == device)
                .ok_or(hr::DSERR_INVALIDCALL)?;
            if !d.initialized {
                return Err(hr::DSERR_UNINITIALIZED);
            }
            if d.cooperative == 0 {
                return Err(hr::DSERR_PRIOLEVELNEEDED);
            }
            if primary
                && let Some(b) = o
                    .buffers
                    .iter_mut()
                    .find(|b| b.device == device && b.primary)
            {
                b.refs += 1;
                return Ok(b.iface);
            }
            let id = o.next;
            o.next += 1;
            let format = format.unwrap_or(d.primary_format);
            let mut voice = Voice::new(format);
            voice.looping = false;
            let buffer = Buffer {
                id,
                device,
                refs: 1,
                iface: new_iface(&BUFFER as *const _ as u64, id),
                notify_iface: new_iface(&NOTIFY as *const _ as u64, id),
                flags: flags | caps::LOCSOFTWARE,
                primary,
                data: vec![if format.bits == 8 { 0x80 } else { 0 }; bytes as usize],
                voice,
                volume: 0,
                pan: 0,
                frequency: format.rate,
                notify: Vec::new(),
                marks: VecDeque::new(),
                play: 0,
                ends_at: None,
            };
            let iface = buffer.iface;
            o.buffers.push(buffer);
            Ok(iface)
        });
        match made {
            Ok(iface) => result(write_pointer(out, iface)),
            Err(e) => e,
        }
    }
}
/// DSCAPS: 24 DWORDs; software mixing only (no hardware buffers).
extern "system" fn ds_get_caps(this: usize, out: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let out = door_windows_abi::widen(out);
    {
        if let Err(e) = with_device(this, |_| Ok(())) {
            return e;
        }
        let mut size = [0u8; 4];
        if read(out, &mut size).is_err() || u32::from_le_bytes(size) != 96 {
            return hr::DSERR_INVALIDPARAM;
        }
        let mut c = [0u32; 24];
        c[0] = 96;
        // PRIMARYMONO|STEREO|8BIT|16BIT, CONTINUOUSRATE, SECONDARYMONO|STEREO|8BIT|16BIT.
        c[1] = 0x0F | 0x10 | 0xF00;
        c[2] = DSBFREQUENCY_MIN;
        c[3] = DSBFREQUENCY_MAX;
        c[4] = 1;
        let bytes: Vec<u8> = c.iter().flat_map(|v| v.to_le_bytes()).collect();
        result(write(out, &bytes))
    }
}
extern "system" fn ds_duplicate_sound_buffer(this: usize, original: usize, out: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let original = door_windows_abi::widen(original);
    let out = door_windows_abi::widen(out);
    {
        if out == 0 {
            return hr::DSERR_INVALIDPARAM;
        }
        let Some(source) = iface_id(original) else {
            return hr::DSERR_INVALIDPARAM;
        };
        let Some(device) = iface_id(this) else {
            return hr::E_POINTER;
        };
        let made = OBJECTS.with(|o| {
            let s = o
                .buffers
                .iter()
                .find(|b| b.id == source && b.device == device)
                .ok_or(hr::DSERR_INVALIDPARAM)?;
            if s.primary {
                return Err(hr::DSERR_INVALIDCALL);
            }
            let id = o.next;
            let mut voice = Voice::new(s.voice.format);
            voice.left = s.voice.left;
            voice.right = s.voice.right;
            voice.step = s.voice.step;
            let copy = Buffer {
                id,
                device,
                refs: 1,
                iface: new_iface(&BUFFER as *const _ as u64, id),
                notify_iface: new_iface(&NOTIFY as *const _ as u64, id),
                flags: s.flags,
                primary: false,
                data: s.data.clone(),
                voice,
                volume: s.volume,
                pan: s.pan,
                frequency: s.frequency,
                notify: Vec::new(),
                marks: VecDeque::new(),
                play: 0,
                ends_at: None,
            };
            o.next += 1;
            let iface = copy.iface;
            o.buffers.push(copy);
            Ok(iface)
        });
        match made {
            Ok(iface) => result(write_pointer(out, iface)),
            Err(e) => e,
        }
    }
}
extern "system" fn ds_set_cooperative_level(this: usize, _window: usize, level: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let level = door_windows_abi::widen(level);
    {
        let level = level as u32;
        if !(DSSCL_NORMAL..=DSSCL_WRITEPRIMARY).contains(&level) {
            return hr::DSERR_INVALIDPARAM;
        }
        // Writing the primary buffer directly is not offered.
        if level == DSSCL_WRITEPRIMARY {
            return hr::DSERR_UNSUPPORTED;
        }
        result(with_device(this, |d| {
            d.cooperative = level;
            Ok(())
        }))
    }
}
extern "system" fn ds_compact(this: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    {
        result(with_device(this, |d| {
            if d.cooperative >= 2 {
                Ok(())
            } else {
                Err(hr::DSERR_PRIOLEVELNEEDED)
            }
        }))
    }
}
/// DSSPEAKER_STEREO with the wide geometry Windows reports for speakers.
extern "system" fn ds_get_speaker_config(this: usize, out: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let out = door_windows_abi::widen(out);
    {
        if let Err(e) = with_device(this, |_| Ok(())) {
            return e;
        }
        result(write_u32(out, 0x0005_0004))
    }
}
extern "system" fn ds_set_speaker_config(this: usize, _config: u32) -> u32 {
    let this = door_windows_abi::widen(this);
    {
        // Windows 7 and later ignore it and return DS_OK.
        result(with_device(this, |_| Ok(())))
    }
}
/// Initializes a device CoCreateInstance made, for the device `guid` names.
extern "system" fn ds_initialize(this: usize, guid: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let guid = door_windows_abi::widen(guid);
    {
        if with_object(this, |d| Ok(d.initialized)) == Ok(true) {
            return hr::DSERR_ALREADYINITIALIZED;
        }
        match is_playback_device(guid) {
            Ok(true) if device_present() => (),
            Ok(_) => return hr::DSERR_NODRIVER,
            Err(e) => return e,
        }
        result(with_object(this, |d| {
            d.initialized = true;
            Ok(())
        }))
    }
}
/// DS_UNCERTIFIED: the driver is not WHQL-certified.
extern "system" fn ds_verify_certification(this: usize, out: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let out = door_windows_abi::widen(out);
    {
        if let Err(e) = with_device(this, |_| Ok(())) {
            return e;
        }
        result(write_u32(out, 1))
    }
}

// ---- IDirectSoundBuffer8 -----------------------------------------------------
#[repr(C)]
struct BufferVtbl {
    query_interface: Fn3,
    add_ref: Fn1,
    release: Fn1,
    get_caps: Fn2,
    get_current_position: Fn3,
    get_format: Fn4,
    get_volume: Fn2,
    get_pan: Fn2,
    get_frequency: Fn2,
    get_status: Fn2,
    initialize: Fn3,
    lock: extern "system" fn(usize, u32, u32, usize, usize, usize, usize, u32) -> u32,
    play: extern "system" fn(usize, u32, u32, u32) -> u32,
    set_current_position: Fn2u,
    set_format: Fn2,
    set_volume: Fn2i,
    set_pan: Fn2i,
    set_frequency: Fn2u,
    stop: Fn1,
    unlock: Fn4Unlock,
    restore: Fn1,
    set_fx: Fn4,
    acquire_resources: Fn4,
    get_object_in_path: extern "system" fn(usize, usize, u32, usize, usize) -> u32,
}
type Fn4Unlock = extern "system" fn(usize, usize, u32, usize, u32) -> u32;
static BUFFER: BufferVtbl = BufferVtbl {
    query_interface: b_query_interface,
    add_ref: b_add_ref,
    release: b_release,
    get_caps: b_get_caps,
    get_current_position: b_get_current_position,
    get_format: b_get_format,
    get_volume: b_get_volume,
    get_pan: b_get_pan,
    get_frequency: b_get_frequency,
    get_status: b_get_status,
    initialize: b_initialize,
    lock: b_lock,
    play: b_play,
    set_current_position: b_set_current_position,
    set_format: b_set_format,
    set_volume: b_set_volume,
    set_pan: b_set_pan,
    set_frequency: b_set_frequency,
    stop: b_stop,
    unlock: b_unlock,
    restore: b_restore,
    set_fx: b_set_fx,
    acquire_resources: b_acquire_resources,
    get_object_in_path: b_get_object_in_path,
};
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
        let found = with_buffer(this, |b, _| {
            Ok(
                if [
                    IID_IUNKNOWN,
                    IID_IDIRECTSOUNDBUFFER,
                    IID_IDIRECTSOUNDBUFFER8,
                ]
                .contains(&iid)
                {
                    Some(b.iface)
                } else if iid == IID_IDIRECTSOUNDNOTIFY
                    && b.flags & caps::CTRLPOSITIONNOTIFY != 0
                    && !b.primary
                {
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
        with_buffer(this, |b, _| {
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
        OBJECTS.with(|o| {
            let Some(at) = o.buffers.iter().position(|b| b.id == id) else {
                return 0;
            };
            o.buffers[at].refs -= 1;
            let left = o.buffers[at].refs;
            if left == 0 {
                // The interface blocks stay allocated: a stale pointer then
                // finds no buffer instead of freed memory.
                o.buffers.remove(at);
            }
            left
        })
    }
}
/// DSBCAPS: dwSize (20), dwFlags, dwBufferBytes, dwUnlockTransferRate,
/// dwPlayCpuOverhead.
extern "system" fn b_get_caps(this: usize, out: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let out = door_windows_abi::widen(out);
    {
        let mut size = [0u8; 4];
        if read(out, &mut size).is_err() || u32::from_le_bytes(size) != 20 {
            return hr::DSERR_INVALIDPARAM;
        }
        let caps = with_buffer(this, |b, d| {
            let bytes = if b.primary {
                d.as_ref().map_or(0, |d| {
                    d.primary_format.rate as usize / 10 * usize::from(d.primary_format.block)
                })
            } else {
                b.data.len()
            };
            Ok([20u32, b.flags, bytes as u32, 0, 0])
        });
        match caps {
            Ok(c) => result(write(
                out,
                &c.iter().flat_map(|v| v.to_le_bytes()).collect::<Vec<u8>>(),
            )),
            Err(e) => e,
        }
    }
}
extern "system" fn b_get_current_position(this: usize, play: usize, write_cursor: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let play = door_windows_abi::widen(play);
    let write_cursor = door_windows_abi::widen(write_cursor);
    {
        let cursors = with_buffer(this, |b, d| {
            if b.primary {
                let block = d
                    .as_ref()
                    .map_or(4, |d| usize::from(d.primary_format.block));
                let at = d.as_ref().map_or(0, |d| d.mixed as usize % 1024) * block;
                return Ok((at as u32, at as u32));
            }
            let size = b.data.len().max(1);
            let write_at = if b.voice.playing {
                b.voice.byte_position() % size
            } else {
                b.play
            };
            Ok((b.play as u32, write_at as u32))
        });
        match cursors {
            Ok((p, w)) => {
                if play != 0 && write_u32(play, p).is_err() {
                    return hr::DSERR_INVALIDPARAM;
                }
                if write_cursor != 0 && write_u32(write_cursor, w).is_err() {
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
        let format = with_buffer(this, |b, d| {
            Ok(if b.primary {
                d.as_ref()
                    .map_or(Format::DEFAULT_PRIMARY, |d| d.primary_format)
            } else {
                b.voice.format
            })
        });
        let Ok(format) = format else {
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
fn control(b: &Buffer, flag: u32) -> Result<(), u32> {
    if b.flags & flag == 0 {
        Err(hr::DSERR_CONTROLUNAVAIL)
    } else {
        Ok(())
    }
}
extern "system" fn b_get_volume(this: usize, out: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let out = door_windows_abi::widen(out);
    {
        match with_buffer(this, |b, _| control(b, caps::CTRLVOLUME).map(|_| b.volume)) {
            Ok(v) => result(write(out, &v.to_le_bytes())),
            Err(e) => e,
        }
    }
}
extern "system" fn b_get_pan(this: usize, out: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let out = door_windows_abi::widen(out);
    {
        match with_buffer(this, |b, _| control(b, caps::CTRLPAN).map(|_| b.pan)) {
            Ok(v) => result(write(out, &v.to_le_bytes())),
            Err(e) => e,
        }
    }
}
extern "system" fn b_get_frequency(this: usize, out: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let out = door_windows_abi::widen(out);
    {
        match with_buffer(this, |b, _| {
            control(b, caps::CTRLFREQUENCY).map(|_| b.frequency)
        }) {
            Ok(v) => result(write_u32(out, v)),
            Err(e) => e,
        }
    }
}
extern "system" fn b_get_status(this: usize, out: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let out = door_windows_abi::widen(out);
    {
        let status = with_buffer(this, |b, d| {
            let sounding = b.voice.playing
                || b.ends_at.is_some()
                || (b.primary && d.as_ref().is_some_and(|d| d.running));
            let mut s = DSBSTATUS_LOCSOFTWARE;
            if sounding {
                s |= DSBSTATUS_PLAYING;
                if b.voice.looping || b.primary {
                    s |= DSBSTATUS_LOOPING;
                }
            }
            Ok(s)
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
        let regions = with_buffer(this, |b, _| {
            if b.primary {
                return Err(hr::DSERR_PRIOLEVELNEEDED);
            }
            let size = b.data.len();
            let offset = if flags & DSBLOCK_FROMWRITECURSOR != 0 {
                if b.voice.playing {
                    b.voice.byte_position() % size
                } else {
                    b.play
                }
            } else {
                offset as usize
            };
            let bytes = if flags & DSBLOCK_ENTIREBUFFER != 0 {
                size
            } else {
                bytes as usize
            };
            let ((o1, l1), (o2, l2)) =
                lock_regions(offset, bytes, size).ok_or(hr::DSERR_INVALIDPARAM)?;
            let base = b.data.as_mut_ptr() as u64;
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
extern "system" fn b_play(this: usize, _reserved: u32, _priority: u32, flags: u32) -> u32 {
    let this = door_windows_abi::widen(this);
    {
        result(with_buffer(this, |b, d| {
            let d = d.as_mut().ok_or(hr::DSERR_INVALIDCALL)?;
            ensure_stream(d)?;
            if b.primary {
                d.running = true;
                return Ok(());
            }
            b.voice.looping = flags & DSBPLAY_LOOPING != 0;
            if !b.voice.playing {
                // Resume from the play cursor (Stop leaves it in place).
                b.voice.set_byte_position(b.play);
                b.marks.clear();
                b.ends_at = None;
            }
            b.voice.playing = true;
            Ok(())
        }))
    }
}
extern "system" fn b_set_current_position(this: usize, position: u32) -> u32 {
    let this = door_windows_abi::widen(this);
    {
        result(with_buffer(this, |b, _| {
            if b.primary {
                return Err(hr::DSERR_INVALIDCALL);
            }
            let position = position as usize;
            if position >= b.data.len() {
                return Err(hr::DSERR_INVALIDPARAM);
            }
            let position = position - position % usize::from(b.voice.format.block);
            b.voice.set_byte_position(position);
            b.play = position;
            b.marks.clear();
            Ok(())
        }))
    }
}
extern "system" fn b_set_format(this: usize, format: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let format = door_windows_abi::widen(format);
    {
        let mut f = [0u8; 40];
        if read(format, &mut f[..18]).is_err() {
            return hr::DSERR_INVALIDPARAM;
        }
        let Some(parsed) = Format::parse(&f) else {
            return hr::DSERR_BADFORMAT;
        };
        result(with_buffer(this, |b, d| {
            if !b.primary {
                return Err(hr::DSERR_INVALIDCALL);
            }
            let d = d.as_mut().ok_or(hr::DSERR_INVALIDCALL)?;
            if d.cooperative < 2 {
                return Err(hr::DSERR_PRIOLEVELNEEDED);
            }
            d.primary_format = parsed;
            Ok(())
        }))
    }
}
fn apply_gains(b: &mut Buffer) {
    let (l, r) = gains(b.volume, b.pan);
    b.voice.left = l;
    b.voice.right = r;
}
extern "system" fn b_set_volume(this: usize, volume: i32) -> u32 {
    let this = door_windows_abi::widen(this);
    {
        result(with_buffer(this, |b, _| {
            control(b, caps::CTRLVOLUME)?;
            if !(DSBVOLUME_MIN..=0).contains(&volume) {
                return Err(hr::DSERR_INVALIDPARAM);
            }
            b.volume = volume;
            apply_gains(b);
            Ok(())
        }))
    }
}
extern "system" fn b_set_pan(this: usize, pan: i32) -> u32 {
    let this = door_windows_abi::widen(this);
    {
        result(with_buffer(this, |b, _| {
            control(b, caps::CTRLPAN)?;
            if !(-DSBPAN_RIGHT..=DSBPAN_RIGHT).contains(&pan) {
                return Err(hr::DSERR_INVALIDPARAM);
            }
            b.pan = pan;
            apply_gains(b);
            Ok(())
        }))
    }
}
extern "system" fn b_set_frequency(this: usize, frequency: u32) -> u32 {
    let this = door_windows_abi::widen(this);
    {
        result(with_buffer(this, |b, _| {
            control(b, caps::CTRLFREQUENCY)?;
            let rate = if frequency == 0 {
                b.voice.format.rate
            } else {
                frequency
            };
            if !(DSBFREQUENCY_MIN..=DSBFREQUENCY_MAX).contains(&rate) {
                return Err(hr::DSERR_INVALIDPARAM);
            }
            b.frequency = rate;
            b.voice.set_frequency(rate);
            Ok(())
        }))
    }
}
extern "system" fn b_stop(this: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    {
        let events = with_buffer(this, |b, d| {
            if b.primary {
                if let Some(d) = d.as_mut() {
                    d.running = false;
                }
                return Ok(Vec::new());
            }
            let was = b.voice.playing;
            b.voice.playing = false;
            b.marks.clear();
            b.ends_at = None;
            b.voice.set_byte_position(b.play);
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
    {
        // The application wrote the buffer in place.
        result(with_buffer(this, |_, _| Ok(())))
    }
}
extern "system" fn b_restore(this: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    result(with_buffer(this, |_, _| Ok(())))
}
extern "system" fn b_set_fx(this: usize, _count: usize, _descs: usize, _results: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    {
        match with_buffer(this, |_, _| Ok(())) {
            Ok(()) => hr::DSERR_CONTROLUNAVAIL,
            Err(e) => e,
        }
    }
}
extern "system" fn b_acquire_resources(
    this: usize,
    _flags: usize,
    _count: usize,
    _results: usize,
) -> u32 {
    let this = door_windows_abi::widen(this);
    result(with_buffer(this, |_, _| Ok(())))
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

// ---- IDirectSoundNotify ------------------------------------------------------
#[repr(C)]
struct NotifyVtbl {
    query_interface: Fn3,
    add_ref: Fn1,
    release: Fn1,
    set_notification_positions: extern "system" fn(usize, u32, usize) -> u32,
}
static NOTIFY: NotifyVtbl = NotifyVtbl {
    query_interface: b_query_interface,
    add_ref: b_add_ref,
    release: b_release,
    set_notification_positions: n_set_notification_positions,
};
/// DSBPOSITIONNOTIFY: dwOffset (u32), then hEventNotify at offset 8.
/// The DSBPOSITIONNOTIFY array SetNotificationPositions passes.
fn notification_list(count: u32, positions: u64) -> Result<Vec<(u32, u64)>, u32> {
    if count > 0 && positions == 0 {
        return Err(hr::DSERR_INVALIDPARAM);
    }
    // DSBPOSITIONNOTIFY { dwOffset, hEventNotify }: 16 bytes on x64, 8 on
    // x86.
    use door_windows_abi::layout::{self, Field::*};
    const NOTIFY: [layout::Field; 2] = [U32, Pointer];
    const BYTES: usize = layout::size(&NOTIFY);
    let mut list = Vec::new();
    for i in 0..u64::from(count) {
        let mut p = [0u8; BYTES];
        read(positions + i * BYTES as u64, &mut p).map_err(|_| hr::DSERR_INVALIDPARAM)?;
        list.push((
            layout::get(&p, &NOTIFY, 0) as u32,
            layout::get(&p, &NOTIFY, 1),
        ));
    }
    Ok(list)
}
extern "system" fn n_set_notification_positions(this: usize, count: u32, positions: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let positions = door_windows_abi::widen(positions);
    {
        let list = match notification_list(count, positions) {
            Ok(l) => l,
            Err(e) => return e,
        };
        result(with_buffer(this, |b, _| {
            if b.voice.playing {
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

// ---- exports -------------------------------------------------------------------
fn is_playback_device(guid: u64) -> Result<bool, u32> {
    if guid == 0 {
        return Ok(true);
    }
    let g = read_guid(guid).map_err(|_| hr::DSERR_INVALIDPARAM)?;
    Ok([
        DSDEVID_DEFAULT_PLAYBACK,
        DSDEVID_DEFAULT_VOICE_PLAYBACK,
        SPEAKER,
    ]
    .contains(&g))
}
fn create(guid: u64, out: u64, outer: u64) -> u32 {
    if out == 0 {
        return hr::DSERR_INVALIDPARAM;
    }
    let _ = write_pointer(out, 0);
    if outer != 0 {
        return hr::DSERR_NOAGGREGATION;
    }
    match is_playback_device(guid) {
        Ok(true) => (),
        Ok(false) => return hr::DSERR_NODRIVER,
        Err(e) => return e,
    }
    if !device_present() {
        return hr::DSERR_NODRIVER;
    }
    result(write_pointer(out, new_device(true)))
}
/// A device object (IDirectSound8), initialized or waiting for Initialize.
fn new_device(initialized: bool) -> u64 {
    OBJECTS.with(|o| {
        let id = o.next;
        o.next += 1;
        let iface = new_iface(&DIRECT_SOUND as *const _ as u64, id);
        o.devices.push(Device {
            id,
            refs: 1,
            initialized,
            cooperative: 0,
            primary_format: Format::DEFAULT_PRIMARY,
            stream: None,
            mixed: 0,
            queued: VecDeque::new(),
            running: false,
            pending: None,
        });
        iface
    })
}
#[unsafe(no_mangle)]
pub extern "system" fn DirectSoundCreate(guid: usize, out: usize, outer: usize) -> u32 {
    let guid = door_windows_abi::widen(guid);
    let out = door_windows_abi::widen(out);
    let outer = door_windows_abi::widen(outer);
    create(guid, out, outer)
}
#[unsafe(no_mangle)]
pub extern "system" fn DirectSoundCreate8(guid: usize, out: usize, outer: usize) -> u32 {
    let guid = door_windows_abi::widen(guid);
    let out = door_windows_abi::widen(out);
    let outer = door_windows_abi::widen(outer);
    create(guid, out, outer)
}
type EnumW = extern "system" fn(usize, usize, usize, usize) -> i32;
fn enumerate(callback: u64, context: u64, wide: bool) -> u32 {
    let devices: &[(Option<&Guid>, &str, &str)] = &[
        (None, "Primary Sound Driver", ""),
        (
            Some(&SPEAKER),
            SPEAKER_NAME,
            "{0.0.0.00000000}.{d0025d01-5ea4-4a6b-9c11-2b6e15500001}",
        ),
    ];
    let shown = if device_present() {
        devices
    } else {
        &devices[..0]
    };
    enumerate_devices(callback, context, wide, shown)
}
/// Calls an LPDSENUMCALLBACK(W/A) with each device (GUID, description,
/// module) until it answers FALSE.
fn enumerate_devices(
    callback: u64,
    context: u64,
    wide: bool,
    devices: &[(Option<&Guid>, &str, &str)],
) -> u32 {
    if callback == 0 {
        return hr::DSERR_INVALIDPARAM;
    }
    // SAFETY: the application passes its LPDSENUMCALLBACK.
    let call: EnumW = unsafe { core::mem::transmute::<usize, EnumW>(callback as usize) };
    let text = |s: &str| -> Vec<u8> {
        if wide {
            s.encode_utf16()
                .chain([0])
                .flat_map(u16::to_le_bytes)
                .collect()
        } else {
            s.bytes().chain([0]).collect()
        }
    };
    for (guid, name, module) in devices {
        let (name, module) = (text(name), text(module));
        let guid = guid.map_or(0, |g| g.as_ptr() as u64);
        if call(
            door_windows_abi::narrow(guid),
            door_windows_abi::narrow(name.as_ptr() as u64),
            door_windows_abi::narrow(module.as_ptr() as u64),
            door_windows_abi::narrow(context),
        ) == 0
        {
            break;
        }
    }
    hr::DS_OK
}
#[unsafe(no_mangle)]
pub extern "system" fn DirectSoundEnumerateW(callback: usize, context: usize) -> u32 {
    let callback = door_windows_abi::widen(callback);
    let context = door_windows_abi::widen(context);
    enumerate(callback, context, true)
}
#[unsafe(no_mangle)]
pub extern "system" fn DirectSoundEnumerateA(callback: usize, context: usize) -> u32 {
    let callback = door_windows_abi::widen(callback);
    let context = door_windows_abi::widen(context);
    enumerate(callback, context, false)
}
/// The real device behind the default-device GUIDs.
#[unsafe(no_mangle)]
pub extern "system" fn GetDeviceID(source: usize, out: usize) -> u32 {
    let source = door_windows_abi::widen(source);
    let out = door_windows_abi::widen(out);
    {
        if out == 0 {
            return hr::DSERR_INVALIDPARAM;
        }
        if source != 0
            && read_guid(source)
                .is_ok_and(|g| capture::is_default_capture(&g) || g == capture::MICROPHONE)
        {
            return result(write(out, &capture::MICROPHONE));
        }
        match is_playback_device(source) {
            Ok(true) => result(write(out, &SPEAKER)),
            Ok(false) => hr::DSERR_NODRIVER,
            Err(e) => e,
        }
    }
}
// DllGetClassObject and DllCanUnloadNow are in `factory`.
