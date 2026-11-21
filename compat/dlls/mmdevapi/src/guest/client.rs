//! What an endpoint activates: IAudioClient (shared mode) with
//! IAudioRenderClient, IAudioCaptureClient, IAudioClock and
//! ISimpleAudioVolume, and IAudioEndpointVolume; endpoint notifications;
//! and the worker thread.
//!
//! A render stream's released frames wait in a queue here; every 5 ms the
//! worker hands what has accumulated to a door.audio stream opened in the
//! client's format (float as 16-bit; door.audio resamples), and reads back
//! how much it played, which is what GetCurrentPadding and IAudioClock
//! answer. A capture stream is a door.audio capture ring converted with
//! winmm's recorder into one-period packets. Event-driven streams have
//! their event set every 10 ms period while running. The worker also
//! watches the device: when door.audio loses it, every stream answers
//! AUDCLNT_E_DEVICE_INVALIDATED and registered IMMNotificationClients hear
//! of the state change and the new default.
use super::*;
use alloc::{collections::VecDeque, vec, vec::Vec};
use door_service_protocol::audio::{self as proto, capture as ring};
use winmm::{Recorder, Wave};

const IID_IAUDIOCLIENT: Guid = guid(
    0x1CB9_AD4C,
    0xDBFA,
    0x4C32,
    [0xB1, 0x78, 0xC2, 0xF5, 0x68, 0xA7, 0x03, 0xB2],
);
const IID_IAUDIORENDERCLIENT: Guid = guid(
    0xF294_ACFC,
    0x3146,
    0x4483,
    [0xA7, 0xBF, 0xAD, 0xDC, 0xA7, 0xC2, 0x60, 0xE2],
);
const IID_IAUDIOCAPTURECLIENT: Guid = guid(
    0xC8AD_BD64,
    0xE71E,
    0x48A0,
    [0xA4, 0xDE, 0x18, 0x5C, 0x39, 0x5C, 0xD3, 0x17],
);
const IID_IAUDIOCLOCK: Guid = guid(
    0xCD63_314F,
    0x3FBA,
    0x4A1B,
    [0x81, 0x2C, 0xEF, 0x96, 0x35, 0x87, 0x28, 0xE7],
);
const IID_ISIMPLEAUDIOVOLUME: Guid = guid(
    0x87CE_5498,
    0x68D6,
    0x44E5,
    [0x92, 0x15, 0x6D, 0xA4, 0x7E, 0xF8, 0x83, 0xD8],
);
const IID_IAUDIOENDPOINTVOLUME: Guid = guid(
    0x5CDF_2C82,
    0x841E,
    0x4546,
    [0x97, 0x22, 0x0C, 0xF7, 0x40, 0x78, 0x22, 0x9A],
);
/// A capture ring of 400 ms.
const RING_BYTES: usize = 40 * ring::PERIOD_BYTES;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Service {
    Render,
    Capture,
    Clock,
    Simple,
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum State {
    Fresh,
    Ready,
    /// Initialized twice: Windows keeps answering, but the stream is gone.
    Broken,
}
struct Client {
    id: u32,
    endpoint: Endpoint,
    state: State,
    wave: Option<Wave>,
    block: usize,
    frames: u32,
    event_mode: bool,
    event: u64,
    running: bool,
    /// The device went away.
    invalid: bool,
    next_event: u64,
    /// GetBuffer's region, and the frames of the GetBuffer not yet released.
    stage: Vec<u8>,
    outstanding: Option<u32>,
    // Render: released frames not yet handed over, frames released and
    // played since the last Reset, the door.audio stream and a section it
    // refused.
    queue: VecDeque<u8>,
    written: u64,
    played: u64,
    stream: Option<u32>,
    pending: Option<(Vec<u8>, u64)>,
    // Capture: the ring (section, address, mapped), the capture stream,
    // converted frames and the position of the first one.
    ring: Option<(u64, u64, u64)>,
    capture: Option<u32>,
    recorder: Option<Recorder>,
    captured: VecDeque<u8>,
    position: u64,
    discontinuity: bool,
}
static CLIENTS: Lock<Vec<Client>> = Lock::new(Vec::new());
static NEXT_ID: AtomicU32 = AtomicU32::new(1);
/// The process's audio session: ISimpleAudioVolume's level and mute.
static SESSION: Lock<(f32, bool)> = Lock::new((1.0, false));
/// Registered IMMNotificationClients.
static NOTIFY: Lock<Vec<u64>> = Lock::new(Vec::new());
static WORKER: AtomicBool = AtomicBool::new(false);

fn with_client<R>(id: u32, f: impl FnOnce(&mut Client) -> Result<R, u32>) -> Result<R, u32> {
    CLIENTS.with(|c| match c.iter_mut().find(|x| x.id == id) {
        Some(x) => f(x),
        None => Err(hr::AUDCLNT_E_DEVICE_INVALIDATED),
    })
}
fn client_id(this: u64) -> u32 {
    match object(this).data {
        Data::Client(id) | Data::Service(_, id) => id,
        _ => 0,
    }
}
/// The client is initialized and its device still there.
fn ready(c: &Client) -> Result<(), u32> {
    match c.state {
        State::Fresh => Err(hr::AUDCLNT_E_NOT_INITIALIZED),
        State::Broken => Err(hr::AUDCLNT_E_DEVICE_INVALIDATED),
        State::Ready if c.invalid => Err(hr::AUDCLNT_E_DEVICE_INVALIDATED),
        State::Ready => Ok(()),
    }
}
pub(super) fn drop_client(id: u32) {
    let gone = CLIENTS.with(|c| c.iter().position(|x| x.id == id).map(|i| c.remove(i)));
    if let Some(c) = gone {
        if let Some(s) = c.stream {
            let _ = audio(|a| a.close(s));
        }
        if let Some(s) = c.capture {
            let _ = audio(|a| a.capture_close(s));
        }
        if let Some((section, address, mapped)) = c.ring {
            release_ring(section, address, mapped);
        }
    }
}
fn session_volume() -> u8 {
    SESSION.with(|s| if s.1 { 0 } else { (s.0 * 100.0 + 0.5) as u8 })
}

// ---- IMMDevice::Activate ------------------------------------------------------------
pub(super) extern "system" fn activate(
    this: usize,
    iid: usize,
    _context: usize,
    _params: usize,
    out: usize,
) -> u32 {
    let this = door_windows_abi::widen(this);
    let iid = door_windows_abi::widen(iid);
    let out = door_windows_abi::widen(out);
    {
        if out == 0 {
            return hr::E_POINTER;
        }
        let _ = write_pointer(out, 0);
        let e = device(this);
        let g = match read_guid(iid) {
            Ok(g) => g,
            Err(e) => return e,
        };
        if g != IID_IAUDIOCLIENT && g != IID_IAUDIOENDPOINTVOLUME {
            return hr::E_NOINTERFACE;
        }
        if !e.present() {
            return hr::AUDCLNT_E_DEVICE_INVALIDATED;
        }
        let p = if g == IID_IAUDIOCLIENT {
            let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
            CLIENTS.with(|c| {
                c.push(Client {
                    id,
                    endpoint: e,
                    state: State::Fresh,
                    wave: None,
                    block: 0,
                    frames: 0,
                    event_mode: false,
                    event: 0,
                    running: false,
                    invalid: false,
                    next_event: 0,
                    stage: Vec::new(),
                    outstanding: None,
                    queue: VecDeque::new(),
                    written: 0,
                    played: 0,
                    stream: None,
                    pending: None,
                    ring: None,
                    capture: None,
                    recorder: None,
                    captured: VecDeque::new(),
                    position: 0,
                    discontinuity: false,
                })
            });
            new_object(&CLIENT_VTBL as *const _ as u64, 0, Data::Client(id))
        } else {
            new_object(&VOLUME_VTBL as *const _ as u64, 0, Data::Volume(e))
        };
        start_worker();
        result(write_pointer(out, p))
    }
}

// ---- IAudioClient -------------------------------------------------------------------
type Init = extern "system" fn(usize, u32, u32, isize, isize, usize, usize) -> u32;
type Format4 = extern "system" fn(usize, u32, usize, usize) -> u32;
#[repr(C)]
struct ClientVtbl {
    query: Fn3,
    add_ref: Fn1,
    release: Fn1,
    initialize: Init,
    buffer_size: Fn2,
    latency: Fn2,
    padding: Fn2,
    supported: Format4,
    mix_format: Fn2,
    period: Fn3,
    start: Fn1,
    stop: Fn1,
    reset: Fn1,
    set_event: Fn2,
    service: Fn3,
}
static CLIENT_VTBL: ClientVtbl = ClientVtbl {
    query: client_query,
    add_ref: obj_add_ref,
    release: obj_release,
    initialize,
    buffer_size,
    latency,
    padding,
    supported,
    mix_format: get_mix_format,
    period,
    start,
    stop,
    reset,
    set_event,
    service,
};
extern "system" fn client_query(this: usize, iid: usize, out: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let iid = door_windows_abi::widen(iid);
    let out = door_windows_abi::widen(out);
    query(this, iid, out, &[IID_IAUDIOCLIENT], &[])
}
/// A WAVEFORMATEX(TENSIBLE) at `p`: its 18 bytes and cbSize more (at most
/// the 22 of WAVEFORMATEXTENSIBLE).
fn read_format(p: u64) -> Result<Vec<u8>, u32> {
    let mut f = vec![0u8; 18];
    read(p, &mut f)?;
    let extra = usize::from(u16::from_le_bytes([f[16], f[17]])).min(22);
    if extra > 0 && u16::from_le_bytes([f[0], f[1]]) == 0xFFFE {
        f.resize(18 + extra, 0);
        read(p, &mut f)?;
    }
    Ok(f)
}
extern "system" fn initialize(
    this: usize,
    mode: u32,
    stream_flags: u32,
    duration: isize,
    _period: isize,
    format: usize,
    _session: usize,
) -> u32 {
    let this = door_windows_abi::widen(this);
    let format = door_windows_abi::widen(format);
    let duration = duration as i64;
    {
        let id = client_id(this);
        if format == 0 {
            return hr::E_POINTER;
        }
        let f = match read_format(format) {
            Ok(f) => f,
            Err(e) => return e,
        };
        let r = with_client(id, |c| {
            match c.state {
                State::Fresh => {}
                State::Ready => {
                    c.state = State::Broken;
                    return Err(hr::AUDCLNT_E_ALREADY_INITIALIZED);
                }
                State::Broken => return Err(hr::AUDCLNT_E_ALREADY_INITIALIZED),
            }
            match mode {
                0 => {}
                1 => return Err(hr::AUDCLNT_E_EXCLUSIVE_MODE_NOT_ALLOWED),
                _ => return Err(hr::E_INVALIDARG),
            }
            if stream_flags & !flags::VALID != 0 {
                return Err(hr::E_INVALIDARG);
            }
            if stream_flags & flags::LOOPBACK != 0 {
                return Err(if c.endpoint == Endpoint::Capture {
                    hr::AUDCLNT_E_WRONG_ENDPOINT_TYPE
                } else {
                    hr::E_NOTIMPL
                });
            }
            let wave = match support(&f) {
                Support::Yes => Wave::parse(&f),
                Support::Closest if stream_flags & flags::AUTOCONVERTPCM != 0 => Wave::parse(&f),
                _ => None,
            }
            .ok_or(hr::AUDCLNT_E_UNSUPPORTED_FORMAT)?;
            if c.invalid {
                return Err(hr::AUDCLNT_E_DEVICE_INVALIDATED);
            }
            c.wave = Some(wave);
            c.block = usize::from(wave.block_align);
            c.frames = buffer_frames(wave.rate, duration);
            c.event_mode = stream_flags & flags::EVENTCALLBACK != 0;
            c.stage = vec![0u8; c.frames as usize * c.block];
            if c.endpoint == Endpoint::Capture {
                let (section, address, mapped) = create_ring().ok_or(hr::E_OUTOFMEMORY)?;
                let opened = audio(|a| a.capture_open(section, &executable_name()));
                let capture = match opened {
                    Some(Ok(s)) => s,
                    Some(Err(door_service_client::Error::Service(
                        door_service_protocol::Code::Denied,
                    ))) => {
                        release_ring(section, address, mapped);
                        return Err(hr::E_ACCESSDENIED);
                    }
                    _ => {
                        release_ring(section, address, mapped);
                        return Err(hr::AUDCLNT_E_DEVICE_INVALIDATED);
                    }
                };
                c.ring = Some((section, address, mapped));
                c.capture = Some(capture);
                c.recorder = Some(Recorder::new(wave));
            }
            c.state = State::Ready;
            Ok(())
        });
        result(r)
    }
}
extern "system" fn buffer_size(this: usize, out: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let out = door_windows_abi::widen(out);
    {
        let r = with_client(client_id(this), |c| match c.state {
            State::Fresh => Err(hr::AUDCLNT_E_NOT_INITIALIZED),
            _ => Ok(c.frames),
        });
        match r {
            Ok(n) => result(write_u32(out, n)),
            Err(e) => e,
        }
    }
}
extern "system" fn latency(this: usize, out: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let out = door_windows_abi::widen(out);
    {
        match with_client(client_id(this), |c| ready(c)) {
            Ok(()) => result(write_u64(out, 0)),
            Err(e) => e,
        }
    }
}
/// Frames a capture packet holds: one 10 ms period.
fn period_frames(c: &Client) -> usize {
    c.wave.map_or(480, |w| (w.rate / 100) as usize)
}
fn available(c: &Client) -> usize {
    (c.captured.len() / c.block.max(1)).min(period_frames(c))
}
extern "system" fn padding(this: usize, out: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let out = door_windows_abi::widen(out);
    {
        let r = with_client(client_id(this), |c| {
            ready(c)?;
            Ok(match c.endpoint {
                Endpoint::Render => c.written.saturating_sub(c.played) as u32,
                Endpoint::Capture => available(c) as u32,
            })
        });
        match r {
            Ok(n) => result(write_u32(out, n)),
            Err(e) => e,
        }
    }
}
extern "system" fn supported(_this: usize, mode: u32, format: usize, closest: usize) -> u32 {
    let format = door_windows_abi::widen(format);
    let closest = door_windows_abi::widen(closest);
    {
        if format == 0 {
            return hr::E_POINTER;
        }
        if closest != 0 {
            let _ = write_pointer(closest, 0);
        }
        let f = match read_format(format) {
            Ok(f) => f,
            Err(e) => return e,
        };
        if mode == 1 {
            return hr::AUDCLNT_E_UNSUPPORTED_FORMAT;
        }
        if mode != 0 {
            return hr::E_INVALIDARG;
        }
        match support(&f) {
            Support::Yes => hr::S_OK,
            Support::Closest => {
                if closest == 0 {
                    return hr::E_POINTER;
                }
                match task_copy(&mix_format()) {
                    Ok(p) => {
                        let _ = write_pointer(closest, p);
                        hr::S_FALSE
                    }
                    Err(e) => e,
                }
            }
            Support::No => hr::AUDCLNT_E_UNSUPPORTED_FORMAT,
        }
    }
}
extern "system" fn get_mix_format(_this: usize, out: usize) -> u32 {
    let out = door_windows_abi::widen(out);
    {
        if out == 0 {
            return hr::E_POINTER;
        }
        match task_copy(&mix_format()) {
            Ok(p) => result(write_pointer(out, p)),
            Err(e) => e,
        }
    }
}
extern "system" fn period(_this: usize, default: usize, minimum: usize) -> u32 {
    let default = door_windows_abi::widen(default);
    let minimum = door_windows_abi::widen(minimum);
    {
        if default == 0 && minimum == 0 {
            return hr::E_POINTER;
        }
        if default != 0 {
            let _ = write_u64(default, PERIOD_DEFAULT as u64);
        }
        if minimum != 0 {
            let _ = write_u64(minimum, PERIOD_MINIMUM as u64);
        }
        hr::S_OK
    }
}
extern "system" fn start(this: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    {
        let r = with_client(client_id(this), |c| {
            ready(c)?;
            if c.running {
                return Err(hr::AUDCLNT_E_NOT_STOPPED);
            }
            if c.event_mode && c.event == 0 {
                return Err(hr::AUDCLNT_E_EVENTHANDLE_NOT_SET);
            }
            if let Some(s) = c.stream {
                let _ = audio(|a| a.pause(s, false));
            }
            c.running = true;
            c.next_event = now() + 10;
            Ok(())
        });
        result(r)
    }
}
extern "system" fn stop(this: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    {
        let r = with_client(client_id(this), |c| {
            ready(c)?;
            if !c.running {
                return Ok(hr::S_FALSE);
            }
            c.running = false;
            if let Some(s) = c.stream {
                let _ = audio(|a| a.pause(s, true));
            }
            Ok(hr::S_OK)
        });
        r.unwrap_or_else(|e| e)
    }
}
extern "system" fn reset(this: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    {
        let r = with_client(client_id(this), |c| {
            ready(c)?;
            if c.running {
                return Err(hr::AUDCLNT_E_NOT_STOPPED);
            }
            if c.outstanding.is_some() {
                return Err(hr::AUDCLNT_E_BUFFER_OPERATION_PENDING);
            }
            if let Some(s) = c.stream.take() {
                let _ = audio(|a| a.close(s));
            }
            c.queue.clear();
            c.pending = None;
            c.written = 0;
            c.played = 0;
            c.captured.clear();
            c.position = 0;
            Ok(())
        });
        result(r)
    }
}
extern "system" fn set_event(this: usize, event: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let event = door_windows_abi::widen(event);
    {
        let r = with_client(client_id(this), |c| {
            if c.state == State::Fresh {
                return Err(hr::AUDCLNT_E_NOT_INITIALIZED);
            }
            if !c.event_mode {
                return Err(hr::AUDCLNT_E_EVENTHANDLE_NOT_EXPECTED);
            }
            if event == 0 {
                return Err(hr::E_INVALIDARG);
            }
            c.event = event;
            Ok(())
        });
        result(r)
    }
}
extern "system" fn service(this: usize, iid: usize, out: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let iid = door_windows_abi::widen(iid);
    let out = door_windows_abi::widen(out);
    {
        if out == 0 {
            return hr::E_POINTER;
        }
        let _ = write_pointer(out, 0);
        let g = match read_guid(iid) {
            Ok(g) => g,
            Err(e) => return e,
        };
        let id = client_id(this);
        let r = with_client(id, |c| {
            if c.state != State::Ready {
                return Err(hr::AUDCLNT_E_NOT_INITIALIZED);
            }
            let (kind, table) = match g {
                IID_IAUDIORENDERCLIENT => (Service::Render, &RENDER_VTBL as *const _ as u64),
                IID_IAUDIOCAPTURECLIENT => (Service::Capture, &CAPTURE_VTBL as *const _ as u64),
                IID_IAUDIOCLOCK => (Service::Clock, &CLOCK_VTBL as *const _ as u64),
                IID_ISIMPLEAUDIOVOLUME => (Service::Simple, &SIMPLE_VTBL as *const _ as u64),
                _ => return Err(hr::E_NOINTERFACE),
            };
            let wrong = match kind {
                Service::Render => c.endpoint != Endpoint::Render,
                Service::Capture => c.endpoint != Endpoint::Capture,
                _ => false,
            };
            if wrong {
                return Err(hr::AUDCLNT_E_WRONG_ENDPOINT_TYPE);
            }
            Ok(table)
        });
        match r {
            Ok(table) => {
                add_ref(this);
                result(write_pointer(
                    out,
                    new_object(table, 0, Data::Service(this, id)),
                ))
            }
            Err(e) => e,
        }
    }
}
fn service_query(this: u64, iid: u64, out: u64, own: Guid) -> u32 {
    query(this, iid, out, &[own], &[])
}

// ---- IAudioRenderClient ---------------------------------------------------------------
#[repr(C)]
struct RenderVtbl {
    query: Fn3,
    add_ref: Fn1,
    release: Fn1,
    get_buffer: Fn3,
    release_buffer: Fn3,
}
static RENDER_VTBL: RenderVtbl = RenderVtbl {
    query: render_query,
    add_ref: obj_add_ref,
    release: obj_release,
    get_buffer: render_get,
    release_buffer: render_release,
};
extern "system" fn render_query(this: usize, iid: usize, out: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let iid = door_windows_abi::widen(iid);
    let out = door_windows_abi::widen(out);
    service_query(this, iid, out, IID_IAUDIORENDERCLIENT)
}
extern "system" fn render_get(this: usize, frames: usize, out: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let frames = door_windows_abi::widen(frames);
    let out = door_windows_abi::widen(out);
    {
        if out == 0 {
            return hr::E_POINTER;
        }
        let frames = frames as u32;
        let r = with_client(client_id(this), |c| {
            ready(c)?;
            if c.outstanding.is_some() {
                return Err(hr::AUDCLNT_E_OUT_OF_ORDER);
            }
            let free = u64::from(c.frames).saturating_sub(c.written.saturating_sub(c.played));
            if u64::from(frames) > free {
                return Err(hr::AUDCLNT_E_BUFFER_TOO_LARGE);
            }
            c.outstanding = Some(frames);
            Ok(c.stage.as_mut_ptr() as u64)
        });
        match r {
            Ok(p) => result(write_pointer(out, p)),
            Err(e) => {
                let _ = write_pointer(out, 0);
                e
            }
        }
    }
}
extern "system" fn render_release(this: usize, frames: usize, buffer_flags: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let frames = door_windows_abi::widen(frames);
    let buffer_flags = door_windows_abi::widen(buffer_flags);
    {
        let frames = frames as u32;
        let r = with_client(client_id(this), |c| {
            ready(c)?;
            let Some(asked) = c.outstanding else {
                return Err(hr::AUDCLNT_E_OUT_OF_ORDER);
            };
            if frames > asked {
                return Err(hr::AUDCLNT_E_INVALID_SIZE);
            }
            let n = frames as usize * c.block;
            if buffer_flags as u32 & AUDCLNT_BUFFERFLAGS_SILENT != 0 {
                // 8-bit PCM's silence is 0x80.
                let zero = if c.wave.is_some_and(|w| w.bits == 8) {
                    0x80
                } else {
                    0
                };
                c.queue.extend(core::iter::repeat_n(zero, n));
            } else {
                c.queue.extend(c.stage[..n].iter().copied());
            }
            c.written += u64::from(frames);
            c.outstanding = None;
            Ok(())
        });
        result(r)
    }
}

// ---- IAudioCaptureClient ------------------------------------------------------------------
type Get6 = extern "system" fn(usize, usize, usize, usize, usize, usize) -> u32;
#[repr(C)]
struct CaptureVtbl {
    query: Fn3,
    add_ref: Fn1,
    release: Fn1,
    get_buffer: Get6,
    release_buffer: Fn2,
    next_packet: Fn2,
}
static CAPTURE_VTBL: CaptureVtbl = CaptureVtbl {
    query: capture_query,
    add_ref: obj_add_ref,
    release: obj_release,
    get_buffer: capture_get,
    release_buffer: capture_release,
    next_packet: capture_next,
};
extern "system" fn capture_query(this: usize, iid: usize, out: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let iid = door_windows_abi::widen(iid);
    let out = door_windows_abi::widen(out);
    service_query(this, iid, out, IID_IAUDIOCAPTURECLIENT)
}
extern "system" fn capture_get(
    this: usize,
    data: usize,
    frames: usize,
    flags_out: usize,
    position: usize,
    qpc: usize,
) -> u32 {
    let this = door_windows_abi::widen(this);
    let data = door_windows_abi::widen(data);
    let frames = door_windows_abi::widen(frames);
    let flags_out = door_windows_abi::widen(flags_out);
    let position = door_windows_abi::widen(position);
    let qpc = door_windows_abi::widen(qpc);
    {
        if data == 0 || frames == 0 || flags_out == 0 {
            return hr::E_POINTER;
        }
        let r = with_client(client_id(this), |c| {
            ready(c)?;
            if c.outstanding.is_some() {
                return Err(hr::AUDCLNT_E_OUT_OF_ORDER);
            }
            let n = available(c);
            if n == 0 {
                return Ok((0u64, 0u32, 0u32, c.position));
            }
            let bytes = n * c.block;
            for (slot, b) in c.stage.iter_mut().zip(c.captured.iter().take(bytes)) {
                *slot = *b;
            }
            let flags = if core::mem::take(&mut c.discontinuity) {
                AUDCLNT_BUFFERFLAGS_DATA_DISCONTINUITY
            } else {
                0
            };
            c.outstanding = Some(n as u32);
            Ok((c.stage.as_mut_ptr() as u64, n as u32, flags, c.position))
        });
        match r {
            Ok((pointer, n, f, at)) => {
                let _ = write_pointer(data, pointer);
                let _ = write_u32(frames, n);
                let _ = write_u32(flags_out, f);
                if position != 0 {
                    let _ = write_u64(position, at);
                }
                if qpc != 0 {
                    let _ = write_u64(qpc, now() * 10_000);
                }
                if n == 0 {
                    hr::AUDCLNT_S_BUFFER_EMPTY
                } else {
                    hr::S_OK
                }
            }
            Err(e) => e,
        }
    }
}
extern "system" fn capture_release(this: usize, frames: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let frames = door_windows_abi::widen(frames);
    {
        let frames = frames as u32;
        let r = with_client(client_id(this), |c| {
            ready(c)?;
            let Some(packet) = c.outstanding else {
                return Err(hr::AUDCLNT_E_OUT_OF_ORDER);
            };
            if frames != 0 && frames != packet {
                return Err(hr::AUDCLNT_E_INVALID_SIZE);
            }
            c.captured.drain(..frames as usize * c.block);
            c.position += u64::from(frames);
            c.outstanding = None;
            Ok(())
        });
        result(r)
    }
}
extern "system" fn capture_next(this: usize, out: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let out = door_windows_abi::widen(out);
    {
        match with_client(client_id(this), |c| ready(c).map(|_| available(c) as u32)) {
            Ok(n) => result(write_u32(out, n)),
            Err(e) => e,
        }
    }
}

// ---- IAudioClock ------------------------------------------------------------------------
#[repr(C)]
struct ClockVtbl {
    query: Fn3,
    add_ref: Fn1,
    release: Fn1,
    frequency: Fn2,
    position: Fn3,
    characteristics: Fn2,
}
static CLOCK_VTBL: ClockVtbl = ClockVtbl {
    query: clock_query,
    add_ref: obj_add_ref,
    release: obj_release,
    frequency: clock_frequency,
    position: clock_position,
    characteristics: clock_characteristics,
};
extern "system" fn clock_query(this: usize, iid: usize, out: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let iid = door_windows_abi::widen(iid);
    let out = door_windows_abi::widen(out);
    service_query(this, iid, out, IID_IAUDIOCLOCK)
}
/// The clock counts bytes of the stream's format.
extern "system" fn clock_frequency(this: usize, out: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let out = door_windows_abi::widen(out);
    {
        let r = with_client(client_id(this), |c| {
            ready(c)?;
            Ok(c.wave
                .map_or(0, |w| u64::from(w.rate) * u64::from(w.block_align)))
        });
        match r {
            Ok(f) => result(write_u64(out, f)),
            Err(e) => e,
        }
    }
}
extern "system" fn clock_position(this: usize, out: usize, qpc: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let out = door_windows_abi::widen(out);
    let qpc = door_windows_abi::widen(qpc);
    {
        if out == 0 {
            return hr::E_POINTER;
        }
        let r = with_client(client_id(this), |c| {
            ready(c)?;
            Ok(match c.endpoint {
                Endpoint::Render => c.played * c.block as u64,
                Endpoint::Capture => c.position * c.block as u64,
            })
        });
        match r {
            Ok(p) => {
                if qpc != 0 {
                    let _ = write_u64(qpc, now() * 10_000);
                }
                result(write_u64(out, p))
            }
            Err(e) => e,
        }
    }
}
extern "system" fn clock_characteristics(_this: usize, out: usize) -> u32 {
    let out = door_windows_abi::widen(out);
    {
        // AUDIOCLOCK_CHARACTERISTIC_FIXED_FREQ.
        result(write_u32(out, 1))
    }
}

// ---- ISimpleAudioVolume --------------------------------------------------------------------
type SetF = extern "system" fn(usize, f32, usize) -> u32;
#[repr(C)]
struct SimpleVtbl {
    query: Fn3,
    add_ref: Fn1,
    release: Fn1,
    set_volume: SetF,
    get_volume: Fn2,
    set_mute: Fn3,
    get_mute: Fn2,
}
static SIMPLE_VTBL: SimpleVtbl = SimpleVtbl {
    query: simple_query,
    add_ref: obj_add_ref,
    release: obj_release,
    set_volume: simple_set_volume,
    get_volume: simple_get_volume,
    set_mute: simple_set_mute,
    get_mute: simple_get_mute,
};
extern "system" fn simple_query(this: usize, iid: usize, out: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let iid = door_windows_abi::widen(iid);
    let out = door_windows_abi::widen(out);
    service_query(this, iid, out, IID_ISIMPLEAUDIOVOLUME)
}
/// Applies the session's level to every render stream of the process.
fn apply_session() {
    let volume = session_volume();
    CLIENTS.with(|c| {
        for s in c.iter().filter_map(|x| x.stream) {
            let _ = audio(|a| a.stream_volume(s, volume));
        }
    });
}
extern "system" fn simple_set_volume(_this: usize, level: f32, _context: usize) -> u32 {
    {
        if !(0.0..=1.0).contains(&level) {
            return hr::E_INVALIDARG;
        }
        SESSION.with(|s| s.0 = level);
        apply_session();
        hr::S_OK
    }
}
extern "system" fn simple_get_volume(_this: usize, out: usize) -> u32 {
    let out = door_windows_abi::widen(out);
    result(write_f32(out, SESSION.with(|s| s.0)))
}
extern "system" fn simple_set_mute(_this: usize, mute: usize, _context: usize) -> u32 {
    let mute = door_windows_abi::widen(mute);
    {
        SESSION.with(|s| s.1 = mute as u32 != 0);
        apply_session();
        hr::S_OK
    }
}
extern "system" fn simple_get_mute(_this: usize, out: usize) -> u32 {
    let out = door_windows_abi::widen(out);
    result(write_u32(out, u32::from(SESSION.with(|s| s.1))))
}

// ---- IAudioEndpointVolume -------------------------------------------------------------------
type ChanSet = extern "system" fn(usize, u32, f32, usize) -> u32;
type Range = extern "system" fn(usize, usize, usize, usize) -> u32;
#[repr(C)]
struct VolumeVtbl {
    query: Fn3,
    add_ref: Fn1,
    release: Fn1,
    register: Fn2,
    unregister: Fn2,
    channel_count: Fn2,
    set_level: SetF,
    set_scalar: SetF,
    get_level: Fn2,
    get_scalar: Fn2,
    set_channel_level: ChanSet,
    set_channel_scalar: ChanSet,
    get_channel_level: Fn3,
    get_channel_scalar: Fn3,
    set_mute: Fn3,
    get_mute: Fn2,
    step_info: Fn3,
    step_up: Fn2,
    step_down: Fn2,
    hardware: Fn2,
    range: Range,
}
static VOLUME_VTBL: VolumeVtbl = VolumeVtbl {
    query: volume_query,
    add_ref: obj_add_ref,
    release: obj_release,
    register: volume_notify,
    unregister: volume_notify,
    channel_count: volume_channels,
    set_level: volume_set_level,
    set_scalar: volume_set_scalar,
    get_level: volume_get_level,
    get_scalar: volume_get_scalar,
    set_channel_level: volume_set_channel_level,
    set_channel_scalar: volume_set_channel_scalar,
    get_channel_level: volume_get_channel_level,
    get_channel_scalar: volume_get_channel_scalar,
    set_mute: volume_set_mute,
    get_mute: volume_get_mute,
    step_info: volume_step_info,
    step_up: volume_step_up,
    step_down: volume_step_down,
    hardware: volume_hardware,
    range: volume_range,
};
extern "system" fn volume_query(this: usize, iid: usize, out: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let iid = door_windows_abi::widen(iid);
    let out = door_windows_abi::widen(out);
    query(this, iid, out, &[IID_IAUDIOENDPOINTVOLUME], &[])
}
extern "system" fn volume_notify(_this: usize, callback: usize) -> u32 {
    let callback = door_windows_abi::widen(callback);
    {
        if callback == 0 {
            hr::E_POINTER
        } else {
            hr::S_OK
        }
    }
}
/// The endpoint's (scalar, mute): door.audio's master output for the
/// speaker, its input level for the microphone.
fn level(e: Endpoint) -> Result<(f32, bool), u32> {
    let s = audio(|a| a.status().ok())
        .flatten()
        .ok_or(hr::AUDCLNT_E_DEVICE_INVALIDATED)?;
    Ok(match e {
        Endpoint::Render => (f32::from(s.output.volume) / 100.0, s.output.muted),
        Endpoint::Capture => (f32::from(s.input.volume) / 100.0, s.input.muted),
    })
}
fn set_level(e: Endpoint, scalar: Option<f32>, mute: Option<bool>) -> u32 {
    if scalar.is_some_and(|s| !(0.0..=1.0).contains(&s)) {
        return hr::E_INVALIDARG;
    }
    match e {
        Endpoint::Render => {
            let Some(Ok(s)) = audio(|a| a.status()) else {
                return hr::AUDCLNT_E_DEVICE_INVALIDATED;
            };
            let mut output = s.output;
            if let Some(v) = scalar {
                output.volume = (v * 100.0 + 0.5) as u8;
            }
            if let Some(m) = mute {
                output.muted = m;
            }
            match audio(|a| a.set_output(output)) {
                Some(Ok(())) => hr::S_OK,
                Some(Err(door_service_client::Error::Service(
                    door_service_protocol::Code::Denied,
                ))) => hr::E_ACCESSDENIED,
                _ => hr::AUDCLNT_E_DEVICE_INVALIDATED,
            }
        }
        Endpoint::Capture => {
            let Some(Ok(s)) = audio(|a| a.status()) else {
                return hr::AUDCLNT_E_DEVICE_INVALIDATED;
            };
            let mut input = s.input;
            if let Some(v) = scalar {
                input.volume = (v * 100.0 + 0.5) as u8;
            }
            if let Some(m) = mute {
                input.muted = m;
            }
            match audio(|a| a.set_input(input)) {
                Some(Ok(())) => hr::S_OK,
                _ => hr::AUDCLNT_E_DEVICE_INVALIDATED,
            }
        }
    }
}
extern "system" fn volume_channels(_this: usize, out: usize) -> u32 {
    let out = door_windows_abi::widen(out);
    result(write_u32(out, 2))
}
extern "system" fn volume_set_level(this: usize, db: f32, _context: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    {
        if !(VOLUME_MIN_DB..=VOLUME_MAX_DB).contains(&db) {
            return hr::E_INVALIDARG;
        }
        set_level(device(this), Some(db_to_scalar(db)), None)
    }
}
extern "system" fn volume_set_scalar(this: usize, scalar: f32, _context: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    set_level(device(this), Some(scalar), None)
}
extern "system" fn volume_get_level(this: usize, out: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let out = door_windows_abi::widen(out);
    {
        match level(device(this)) {
            Ok((s, _)) => result(write_f32(out, scalar_to_db(s))),
            Err(e) => e,
        }
    }
}
extern "system" fn volume_get_scalar(this: usize, out: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let out = door_windows_abi::widen(out);
    {
        match level(device(this)) {
            Ok((s, _)) => result(write_f32(out, s)),
            Err(e) => e,
        }
    }
}
/// Both channels follow the master level (door.audio's balance is the
/// shell's).
extern "system" fn volume_set_channel_level(
    this: usize,
    channel: u32,
    db: f32,
    context: usize,
) -> u32 {
    let this = door_windows_abi::widen(this);
    let context = door_windows_abi::widen(context);
    {
        if channel >= 2 {
            return hr::E_INVALIDARG;
        }
        volume_set_level(
            door_windows_abi::narrow(this),
            db,
            door_windows_abi::narrow(context),
        )
    }
}
extern "system" fn volume_set_channel_scalar(
    this: usize,
    channel: u32,
    scalar: f32,
    context: usize,
) -> u32 {
    let this = door_windows_abi::widen(this);
    let context = door_windows_abi::widen(context);
    {
        if channel >= 2 {
            return hr::E_INVALIDARG;
        }
        volume_set_scalar(
            door_windows_abi::narrow(this),
            scalar,
            door_windows_abi::narrow(context),
        )
    }
}
extern "system" fn volume_get_channel_level(this: usize, channel: usize, out: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let channel = door_windows_abi::widen(channel);
    let out = door_windows_abi::widen(out);
    {
        if channel as u32 >= 2 {
            return hr::E_INVALIDARG;
        }
        volume_get_level(
            door_windows_abi::narrow(this),
            door_windows_abi::narrow(out),
        )
    }
}
extern "system" fn volume_get_channel_scalar(this: usize, channel: usize, out: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let channel = door_windows_abi::widen(channel);
    let out = door_windows_abi::widen(out);
    {
        if channel as u32 >= 2 {
            return hr::E_INVALIDARG;
        }
        volume_get_scalar(
            door_windows_abi::narrow(this),
            door_windows_abi::narrow(out),
        )
    }
}
extern "system" fn volume_set_mute(this: usize, mute: usize, _context: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let mute = door_windows_abi::widen(mute);
    set_level(device(this), None, Some(mute as u32 != 0))
}
extern "system" fn volume_get_mute(this: usize, out: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let out = door_windows_abi::widen(out);
    {
        match level(device(this)) {
            Ok((_, m)) => result(write_u32(out, u32::from(m))),
            Err(e) => e,
        }
    }
}
fn current_step(e: Endpoint) -> Result<u32, u32> {
    let (s, _) = level(e)?;
    Ok((s * (VOLUME_STEPS - 1) as f32 + 0.5) as u32)
}
extern "system" fn volume_step_info(this: usize, step: usize, count: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let step = door_windows_abi::widen(step);
    let count = door_windows_abi::widen(count);
    {
        let at = match current_step(device(this)) {
            Ok(n) => n,
            Err(e) => return e,
        };
        if step != 0 {
            let _ = write_u32(step, at);
        }
        result(write_u32(count, VOLUME_STEPS))
    }
}
fn step_by(this: u64, delta: i32) -> u32 {
    let e = device(this);
    let at = match current_step(e) {
        Ok(n) => n as i32,
        Err(e) => return e,
    };
    let next = (at + delta).clamp(0, VOLUME_STEPS as i32 - 1);
    set_level(e, Some(next as f32 / (VOLUME_STEPS - 1) as f32), None)
}
extern "system" fn volume_step_up(this: usize, _context: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    step_by(this, 1)
}
extern "system" fn volume_step_down(this: usize, _context: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    step_by(this, -1)
}
extern "system" fn volume_hardware(_this: usize, out: usize) -> u32 {
    let out = door_windows_abi::widen(out);
    {
        // ENDPOINT_HARDWARE_SUPPORT_VOLUME | _MUTE.
        result(write_u32(out, 3))
    }
}
extern "system" fn volume_range(_this: usize, min: usize, max: usize, step: usize) -> u32 {
    let min = door_windows_abi::widen(min);
    let max = door_windows_abi::widen(max);
    let step = door_windows_abi::widen(step);
    {
        if min == 0 || max == 0 || step == 0 {
            return hr::E_POINTER;
        }
        let _ = write_f32(min, VOLUME_MIN_DB);
        let _ = write_f32(max, VOLUME_MAX_DB);
        result(write_f32(step, VOLUME_STEP_DB))
    }
}

// ---- endpoint notifications --------------------------------------------------------------
pub(super) extern "system" fn register_notification(_this: usize, callback: usize) -> u32 {
    let callback = door_windows_abi::widen(callback);
    {
        if callback == 0 {
            return hr::E_POINTER;
        }
        call(callback, 1, &[]);
        NOTIFY.with(|n| n.push(callback));
        start_worker();
        hr::S_OK
    }
}
pub(super) extern "system" fn unregister_notification(_this: usize, callback: usize) -> u32 {
    let callback = door_windows_abi::widen(callback);
    {
        if callback == 0 {
            return hr::E_POINTER;
        }
        let found = NOTIFY.with(|n| n.iter().position(|&c| c == callback).map(|i| n.remove(i)));
        match found {
            Some(c) => {
                call(c, 2, &[]);
                hr::S_OK
            }
            None => hr::E_NOTFOUND,
        }
    }
}
/// Calls method `slot` of the application object `p` with `args`.
fn call(p: u64, slot: u64, args: &[u64]) -> u32 {
    let Ok(table) = read_pointer(p) else {
        return hr::E_POINTER;
    };
    let Ok(f) = read_pointer(table + POINTER as u64 * slot) else {
        return hr::E_POINTER;
    };
    let f = f as usize;
    // SAFETY: COM methods of the documented shapes (all integer arguments).
    unsafe {
        match args {
            [] => core::mem::transmute::<usize, extern "system" fn(usize) -> u32>(f)(
                door_windows_abi::narrow(p),
            ),
            [a] => core::mem::transmute::<usize, extern "system" fn(usize, usize) -> u32>(f)(
                door_windows_abi::narrow(p),
                door_windows_abi::narrow(*a),
            ),
            [a, b] => {
                core::mem::transmute::<usize, extern "system" fn(usize, usize, usize) -> u32>(f)(
                    door_windows_abi::narrow(p),
                    door_windows_abi::narrow(*a),
                    door_windows_abi::narrow(*b),
                )
            }
            [a, b, c, ..] => core::mem::transmute::<
                usize,
                extern "system" fn(usize, usize, usize, usize) -> u32,
            >(f)(
                door_windows_abi::narrow(p),
                door_windows_abi::narrow(*a),
                door_windows_abi::narrow(*b),
                door_windows_abi::narrow(*c),
            ),
        }
    }
}
/// Tells every registered client that `e` changed to `state`, and that the
/// default endpoint for each role is now `e` (or none).
fn announce(e: Endpoint, present: bool) {
    let callbacks = NOTIFY.with(|n| n.clone());
    if callbacks.is_empty() {
        return;
    }
    let id: Vec<u16> = e.id().encode_utf16().chain([0]).collect();
    let state = if present {
        DEVICE_STATE_ACTIVE
    } else {
        DEVICE_STATE_NOTPRESENT
    };
    for &c in &callbacks {
        // OnDeviceStateChanged, then OnDefaultDeviceChanged per role.
        call(c, 3, &[id.as_ptr() as u64, u64::from(state)]);
        for role in 0..3u64 {
            let default = if present { id.as_ptr() as u64 } else { 0 };
            call(c, 6, &[u64::from(e.flow()), role, default]);
        }
    }
}

// ---- capture rings ---------------------------------------------------------------------------
fn ring_word(base: u64, at: usize) -> &'static AtomicU32 {
    // SAFETY: the ring stays mapped while its client exists; fields aligned.
    unsafe { &*((base + at as u64) as *const AtomicU32) }
}
fn ring_count(base: u64, at: usize) -> &'static core::sync::atomic::AtomicU64 {
    // SAFETY: as ring_word.
    unsafe { &*((base + at as u64) as *const core::sync::atomic::AtomicU64) }
}
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
    // SAFETY: no reference into the ring outlives its client.
    if unsafe { door_sys::section_unmap(address, mapped) }.is_ok() {
        unsafe { DoorReleaseSectionRange(address, mapped) };
    }
    let _ = door_sys::close(section);
}
/// The process executable's file name (Privacy > Microphone lists it).
fn executable_name() -> String {
    let mut units = [0u16; 260];
    let n = unsafe { DoorGetModuleFileNameW(0, units.as_mut_ptr() as u64, units.len() as u32) }
        as usize;
    let path = String::from_utf16_lossy(&units[..n.min(units.len())]);
    let name = path.rsplit(['\\', '/']).next().unwrap_or("");
    let mut end = name.len().min(proto::NAME_BYTES);
    while !name.is_char_boundary(end) {
        end -= 1;
    }
    String::from(&name[..end])
}
/// A section holding `data`.
fn section(data: &[u8]) -> Option<u64> {
    let mapped = (data.len() as u64).div_ceil(4096).max(1) * 4096;
    let address = unsafe { DoorReserveSectionRange(mapped) };
    if address == 0 {
        return None;
    }
    let Ok(handle) =
        door_sys::section_create(mapped, door_abi::rights::ALL & !door_abi::rights::WAIT)
    else {
        unsafe { DoorReleaseSectionRange(address, mapped) };
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
        unsafe { DoorReleaseSectionRange(address, mapped) };
    }
    if filled.is_err() {
        let _ = door_sys::close(handle);
        return None;
    }
    Some(handle)
}

// ---- the worker ----------------------------------------------------------------------------
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
extern "system" fn worker(_argument: usize) -> u32 {
    {
        let mut present = [true, true];
        let mut next_check = 0u64;
        loop {
            let events = pass();
            for e in events {
                unsafe { DoorSetEvent(e) };
            }
            let t = now();
            if t >= next_check {
                next_check = t + 200;
                let status = audio(|a| a.status().ok()).flatten();
                let now_present = [
                    status.is_some_and(|s| s.present),
                    status.is_some_and(|s| s.present && s.capture),
                ];
                for (k, e) in [Endpoint::Render, Endpoint::Capture]
                    .into_iter()
                    .enumerate()
                {
                    if now_present[k] != present[k] {
                        present[k] = now_present[k];
                        if !now_present[k] {
                            CLIENTS.with(|c| {
                                for x in c.iter_mut().filter(|x| x.endpoint == e) {
                                    x.invalid = true;
                                }
                            });
                        }
                        announce(e, now_present[k]);
                    }
                }
            }
            unsafe { DoorSleep(5) };
        }
    }
}
/// One pass: what each render stream played and the frames to hand over,
/// what each capture ring holds, and the events due.
fn pass() -> Vec<u64> {
    let mut events = Vec::new();
    let t = now();
    let volume = session_volume();
    CLIENTS.with(|clients| {
        for c in clients
            .iter_mut()
            .filter(|c| c.state == State::Ready && !c.invalid)
        {
            let Some(wave) = c.wave else { continue };
            match c.endpoint {
                Endpoint::Render => render_pass(c, wave, volume),
                Endpoint::Capture => capture_pass(c),
            }
            if c.running && c.event_mode && c.event != 0 && t >= c.next_event {
                c.next_event = c.next_event.max(t.saturating_sub(10)) + 10;
                events.push(c.event);
            }
        }
    });
    events
}
fn render_pass(c: &mut Client, wave: Wave, volume: u8) {
    if let Some(s) = c.stream
        && let Some(Ok(status)) = audio(|a| a.stream(s))
    {
        c.played = status.played.min(c.written);
    }
    if !c.running {
        return;
    }
    if c.stream.is_none() && (!c.queue.is_empty() || c.pending.is_some()) {
        let format = proto::Format {
            rate: wave.rate,
            channels: wave.channels as u8,
            bits: wave.stream_bits(),
        };
        match audio(|a| a.open(format)) {
            Some(Ok(s)) => {
                let _ = audio(|a| a.stream_volume(s, volume));
                c.stream = Some(s);
            }
            _ => return,
        }
    }
    let Some(stream) = c.stream else { return };
    loop {
        let (pcm, frames) = match c.pending.take() {
            Some(p) => p,
            None => {
                let whole = c.queue.len() / c.block.max(1) * c.block;
                if whole == 0 {
                    return;
                }
                let bytes: Vec<u8> = c.queue.drain(..whole).collect();
                (wave.convert(&bytes), (whole / c.block) as u64)
            }
        };
        let queued = section(&pcm).is_some_and(|handle| {
            let queued = audio(|a| a.queue(stream, handle, pcm.len()).is_ok()).unwrap_or(false);
            let _ = door_sys::close(handle);
            queued
        });
        if !queued {
            c.pending = Some((pcm, frames));
            return;
        }
    }
}
fn capture_pass(c: &mut Client) {
    let Some((_, base, _)) = c.ring else { return };
    let written = ring_count(base, ring::WRITE).load(Ordering::Acquire);
    let mut read = ring_count(base, ring::READ).load(Ordering::Relaxed);
    let data = base + ring::HEADER as u64;
    while read < written {
        let at = (read % RING_BYTES as u64) as usize;
        let take = ((written - read) as usize)
            .min(RING_BYTES - at)
            .min(ring::PERIOD_BYTES * 10);
        if c.running {
            // SAFETY: the producer finished these bytes (Acquire above).
            let bytes =
                unsafe { core::slice::from_raw_parts((data + at as u64) as *const u8, take) };
            if let Some(r) = c.recorder.as_mut() {
                let mut out = Vec::new();
                r.push(bytes, &mut out);
                c.captured.extend(out);
            }
        }
        read += take as u64;
    }
    ring_count(base, ring::READ).store(read, Ordering::Release);
    // A full buffer drops its oldest frames, as the engine's does.
    let limit = c.frames as usize * c.block;
    if c.captured.len() > limit {
        let over = (c.captured.len() - limit).div_ceil(c.block.max(1)) * c.block;
        c.captured.drain(..over.min(c.captured.len()));
        c.position += (over / c.block.max(1)) as u64;
        c.discontinuity = true;
    }
}
