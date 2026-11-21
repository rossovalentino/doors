//! winmm.dll exports over door.audio. Each open waveOut device is one
//! door.audio stream; the application's buffers are copied into sections
//! (mapped in the process section window ntdll hands out) and queued, at
//! most two at a time with about 100 ms each, by one worker thread that
//! also marks buffers done and runs the callbacks (function, window,
//! thread or event) outside every lock.
use crate::{Wave, mmsyserr as mm};

mod mci;
mod midi;
mod mixer;
use alloc::{boxed::Box, collections::VecDeque, string::String, sync::Arc, vec, vec::Vec};
use core::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use door_service_client::audio::Audio;
use door_service_protocol::audio::{self as proto, Format};

/// A spin lock for the device and timer tables.
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
        // SAFETY: `held` gives this closure the only access.
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
    fn DoorGetTickCount64() -> u64;
    fn DoorSetEvent(handle: u64) -> i32;
    fn DoorGetModuleHandleW(name: u64) -> u64;
    fn DoorGetProcAddress(module: u64, name: u64) -> u64;
    fn DoorCreateFileW(
        path: u64,
        access: u32,
        share: u32,
        security: u64,
        disposition: u32,
        flags: u32,
        template: u64,
    ) -> u64;
    fn DoorReadFile(handle: u64, destination: u64, bytes: u32, read: u64, overlapped: u64) -> i32;
    fn DoorWriteFile(handle: u64, source: u64, bytes: u32, written: u64, overlapped: u64) -> i32;
    fn DoorCloseFile(handle: u64) -> i32;
    fn DoorFindResource(module: u64, kind: u64, name: u64, language: u16) -> u64;
    fn DoorLoadResource(module: u64, handle: u64) -> u64;
    fn DoorSizeofResource(module: u64, handle: u64) -> u32;
    fn DoorGetModuleFileNameW(module: u64, buffer: u64, capacity: u32) -> u32;
}

// ---- application memory ------------------------------------------------
/// One checked copy moves at most this much for a console or windowed
/// application (the kernel's per-role USER_COPY bound).
const COPY_CHUNK: usize = door_windows_abi::APPLICATION_COPY_BYTES;
fn read(pointer: u64, out: &mut [u8]) -> Result<(), u32> {
    if pointer == 0 && !out.is_empty() {
        return Err(mm::INVALPARAM);
    }
    for (i, chunk) in out.chunks_mut(COPY_CHUNK).enumerate() {
        let source = pointer + (i * COPY_CHUNK) as u64;
        if unsafe { DoorUserCopy(chunk.as_mut_ptr() as u64, source, chunk.len()) } != 0 {
            return Err(mm::INVALPARAM);
        }
    }
    Ok(())
}
fn write(pointer: u64, data: &[u8]) -> Result<(), u32> {
    if pointer == 0 && !data.is_empty() {
        return Err(mm::INVALPARAM);
    }
    for (i, chunk) in data.chunks(COPY_CHUNK).enumerate() {
        let destination = pointer + (i * COPY_CHUNK) as u64;
        if unsafe { DoorUserCopy(destination, chunk.as_ptr() as u64, chunk.len()) } != 0 {
            return Err(mm::INVALPARAM);
        }
    }
    Ok(())
}
fn read_u32(pointer: u64) -> Result<u32, u32> {
    let mut b = [0u8; 4];
    read(pointer, &mut b)?;
    Ok(u32::from_le_bytes(b))
}
/// The program's pointer width (P17: 4 for a 32-bit program).
pub(crate) const POINTER: usize = door_windows_abi::layout::POINTER;
/// A pointer-sized program value (pointers, DWORD_PTR), sign-extended on
/// x86.
pub(crate) fn read_pointer(pointer: u64) -> Result<u64, u32> {
    let mut b = [0u8; 8];
    read(pointer, &mut b[..POINTER])?;
    Ok(door_windows_abi::layout::get(
        &b,
        &[door_windows_abi::layout::Field::Pointer],
        0,
    ))
}
pub(crate) fn write_pointer(pointer: u64, value: u64) -> Result<(), u32> {
    write(pointer, &value.to_le_bytes()[..POINTER])
}
/// WAVEHDR (mmsystem.h packs it; its fields fall on natural boundaries
/// anyway): lpData, dwBufferLength, dwBytesRecorded, dwUser, dwFlags,
/// dwLoops, lpNext, reserved (48 bytes on x64, 32 on x86).
const WAVEHDR: [door_windows_abi::layout::Field; 8] = {
    use door_windows_abi::layout::Field::*;
    [Pointer, U32, U32, Pointer, U32, U32, Pointer, Pointer]
};
const WAVEHDR_BYTES: u32 = door_windows_abi::layout::size_pack(&WAVEHDR, 1) as u32;
const WH_LENGTH: u64 = door_windows_abi::layout::offset_pack(&WAVEHDR, 1, 1) as u64;
const WH_RECORDED: u64 = door_windows_abi::layout::offset_pack(&WAVEHDR, 2, 1) as u64;
const WH_FLAGS: u64 = door_windows_abi::layout::offset_pack(&WAVEHDR, 4, 1) as u64;
const WH_LOOPS: u64 = door_windows_abi::layout::offset_pack(&WAVEHDR, 5, 1) as u64;
fn write_u32(pointer: u64, value: u32) -> Result<(), u32> {
    write(pointer, &value.to_le_bytes())
}
fn tick() -> u64 {
    unsafe { DoorGetTickCount64() }
}

// ---- the door.audio connection ------------------------------------------
static AUDIO: Lock<Option<Audio>> = Lock::new(None);
/// Runs `f` on the process's door.audio connection, connecting on first
/// use; None without the service or a sound device.
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
/// A section of `bytes` in the shared section window, filled through its
/// temporary mapping by `fill` (given the mapped address).
fn section(bytes: usize, fill: impl FnOnce(u64)) -> Option<u64> {
    let mapped = (bytes as u64).div_ceil(4096).max(1) * 4096;
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
    // SAFETY: the reserved range is this section's alone; `fill` ends
    // before the unmap and before the handle leaves this process.
    let filled = unsafe {
        door_sys::section_map(
            handle,
            address,
            door_abi::rights::READ | door_abi::rights::WRITE,
        )
    }
    .map(|_| fill(address));
    if filled.is_err() || unsafe { door_sys::section_unmap(address, mapped) }.is_ok() {
        release();
    }
    if filled.is_err() {
        let _ = door_sys::close(handle);
        return None;
    }
    Some(handle)
}

// ---- devices ---------------------------------------------------------------
const WHDR_DONE: u32 = 0x1;
const WHDR_PREPARED: u32 = 0x2;
const WHDR_BEGINLOOP: u32 = 0x4;
const WHDR_ENDLOOP: u32 = 0x8;
const WHDR_INQUEUE: u32 = 0x10;
const WOM_OPEN: u32 = 0x3BB;
const WOM_CLOSE: u32 = 0x3BC;
const WOM_DONE: u32 = 0x3BD;
const CALLBACK_TYPEMASK: u32 = 0x0007_0000;
const CALLBACK_NULL: u32 = 0;
const CALLBACK_WINDOW: u32 = 0x0001_0000;
const CALLBACK_THREAD: u32 = 0x0002_0000;
const CALLBACK_FUNCTION: u32 = 0x0003_0000;
const CALLBACK_EVENT: u32 = 0x0005_0000;
const WAVE_FORMAT_QUERY: u32 = 0x1;
const WAVE_MAPPER: u32 = u32::MAX;

/// Whom a buffer is reported to when it has played.
enum Data {
    /// An application WAVEHDR (its address).
    Header(u64),
    /// PlaySound's own sound.
    Sound,
}
/// Where a buffer's samples are: an application buffer stays the
/// driver's until WHDR_DONE, so it is read (and converted) only when
/// handed to door.audio; PlaySound keeps its own converted copy.
#[derive(Clone)]
enum Source {
    Application { data: u64 },
    Owned(Arc<Vec<u8>>),
}
struct Buffer {
    data: Data,
    source: Source,
    /// Stream bytes of one repetition.
    length: usize,
    /// Repetitions still to hand over (WAVEHDR loops; at least 1).
    loops: u32,
    /// Some repetitions were handed over already.
    started: bool,
}
impl Buffer {
    fn new(data: Data, source: Source, length: usize, loops: u32) -> Self {
        Self {
            data,
            source,
            length,
            loops: loops.max(1),
            started: false,
        }
    }
}
/// Copies one repetition of `source` (`length` stream bytes) to `out`,
/// converting application samples; unreadable application memory plays as
/// silence.
fn fill(wave: &Wave, source: &Source, out: u64, length: usize) {
    match source {
        Source::Owned(bytes) => unsafe {
            core::ptr::copy_nonoverlapping(bytes.as_ptr(), out as *mut u8, length.min(bytes.len()));
        },
        Source::Application { data } if wave.encoding == crate::Encoding::Pcm => {
            if read(*data, unsafe {
                core::slice::from_raw_parts_mut(out as *mut u8, length)
            })
            .is_err()
            {
                unsafe { core::ptr::write_bytes(out as *mut u8, 0, length) };
            }
        }
        Source::Application { data } => {
            // 32-bit float to 16-bit: two caller bytes per stream byte.
            let mut chunk = [0u8; COPY_CHUNK];
            let mut done = 0;
            while done < length {
                let take = (length - done).min(COPY_CHUNK / 2);
                let converted = match read(*data + 2 * done as u64, &mut chunk[..2 * take]) {
                    Ok(()) => wave.convert_into(&chunk[..2 * take], unsafe {
                        core::slice::from_raw_parts_mut((out + done as u64) as *mut u8, take)
                    }),
                    Err(_) => 0,
                };
                if converted < take {
                    unsafe {
                        core::ptr::write_bytes(
                            (out + (done + converted) as u64) as *mut u8,
                            0,
                            take - converted,
                        )
                    };
                }
                done += take;
            }
        }
    }
}
/// Sections door.audio holds per stream at once.
const QUEUED_SECTIONS: usize = 2;
struct Device {
    handle: u64,
    wave: Wave,
    stream: u32,
    callback: (u32, u64, u64),
    pending: VecDeque<Buffer>,
    /// Buffers handed to door.audio: (data, cumulative end frame).
    playing: VecDeque<(Data, u64)>,
    /// End frames of the sections door.audio holds.
    sections: VecDeque<u64>,
    /// Frames handed over to the current stream.
    submitted: u64,
    paused: bool,
    volume: u32,
    /// A PlaySound device: closed when drained (unless looping).
    sound: Option<SoundMode>,
    /// door.audio lost the output device: every buffer came back done and
    /// waveOutWrite answers MMSYSERR_NODRIVER, as Windows does once the
    /// device is gone.
    lost: bool,
}
#[derive(Clone)]
enum SoundMode {
    Once,
    Loop(Arc<Vec<u8>>),
}
static DEVICES: Lock<Vec<Device>> = Lock::new(Vec::new());
static NEXT_HANDLE: AtomicU64 = AtomicU64::new(0x00A0_0010);
static WORKER: AtomicBool = AtomicBool::new(false);
/// Frames door.audio has played on `stream`, or None when it is gone.
fn played(stream: u32) -> Option<(u64, bool)> {
    audio(|a| a.stream(stream).ok().map(|s| (s.played, s.drained))).flatten()
}
/// Hands pending buffers to door.audio: up to two sections of at least
/// 100 ms each (or what is pending).
fn feed(d: &mut Device) {
    let frame = d.wave.stream_frame();
    let target = (d.wave.rate as usize / 10 * frame).min(proto::MAX_SECTION);
    while d.sections.len() < QUEUED_SECTIONS && !d.pending.is_empty() {
        // Plan the section: (repetitions, byte offset where they end) per
        // leading buffer; only the last may be partly taken.
        let mut total = 0usize;
        let mut plan: Vec<(u32, usize)> = Vec::new();
        for b in &d.pending {
            let mut reps = 0;
            while reps < b.loops {
                if b.length == 0 {
                    reps = b.loops;
                    break;
                }
                if total >= target || (total != 0 && total + b.length > proto::MAX_SECTION) {
                    break;
                }
                total += b.length;
                reps += 1;
            }
            if reps > 0 {
                plan.push((reps, total));
            }
            if reps < b.loops || total >= target {
                break;
            }
        }
        if total != 0 {
            let wave = d.wave;
            let pending = &d.pending;
            let Some(handle) = section(total, |address| {
                let mut at = 0;
                for ((reps, _), b) in plan.iter().zip(pending) {
                    for _ in 0..*reps {
                        if b.length != 0 {
                            fill(&wave, &b.source, address + at as u64, b.length);
                            at += b.length;
                        }
                    }
                }
            }) else {
                return;
            };
            let queued = audio(|a| a.queue(d.stream, handle, total).is_ok()).unwrap_or(false);
            let _ = door_sys::close(handle);
            if !queued {
                // door.audio still holds two sections; the next pass retries.
                return;
            }
        }
        let start = d.submitted;
        for (reps, end) in plan {
            let Some(b) = d.pending.front_mut() else {
                break;
            };
            b.loops -= reps;
            b.started = true;
            if b.loops == 0 {
                let b = d.pending.pop_front().unwrap();
                d.playing.push_back((b.data, start + (end / frame) as u64));
            }
        }
        if total != 0 {
            d.submitted += (total / frame) as u64;
            d.sections.push_back(d.submitted);
        }
    }
}
/// One callback to run after the device lock is released.
struct Note {
    device: u64,
    callback: (u32, u64, u64),
    message: u32,
    header: u64,
}
/// The stream is gone (door.audio lost the device): every buffer, played
/// or not, comes back to the application done.
fn lose(d: &mut Device, notes: &mut Vec<Note>) {
    retire(d, u64::MAX, notes);
    while let Some(b) = d.pending.pop_front() {
        if let Data::Header(header) = b.data {
            if let Ok(flags) = read_u32(header + WH_FLAGS) {
                let _ = write_u32(header + WH_FLAGS, (flags | WHDR_DONE) & !WHDR_INQUEUE);
            }
            notes.push(Note {
                device: d.handle,
                callback: d.callback,
                message: WOM_DONE,
                header,
            });
        }
    }
    d.lost = true;
}
fn retire(d: &mut Device, played: u64, notes: &mut Vec<Note>) {
    while d.sections.front().is_some_and(|end| *end <= played) {
        d.sections.pop_front();
    }
    while d.playing.front().is_some_and(|(_, end)| *end <= played) {
        let (data, _) = d.playing.pop_front().unwrap();
        if let Data::Header(header) = data {
            // WHDR_DONE set, WHDR_INQUEUE cleared, in the application's header.
            if let Ok(flags) = read_u32(header + WH_FLAGS) {
                let _ = write_u32(header + WH_FLAGS, (flags | WHDR_DONE) & !WHDR_INQUEUE);
            }
            notes.push(Note {
                device: d.handle,
                callback: d.callback,
                message: WOM_DONE,
                header,
            });
        }
    }
}
/// The callbacks and user32 functions winmm calls, at pointer width (P17):
/// waveOutProc(hwo, uMsg, dwInstance, dwParam1, dwParam2), PostMessageW,
/// PostThreadMessageW.
type WaveProc = extern "system" fn(usize, u32, usize, usize, usize);
type PostMessage = extern "system" fn(usize, u32, usize, isize) -> i32;
type PostThreadMessage = extern "system" fn(u32, u32, usize, isize) -> i32;
fn user32(name: &core::ffi::CStr) -> u64 {
    let module_name: Vec<u16> = "user32.dll".encode_utf16().chain([0]).collect();
    let module = unsafe { DoorGetModuleHandleW(module_name.as_ptr() as u64) };
    if module == 0 {
        0
    } else {
        unsafe { DoorGetProcAddress(module, name.as_ptr() as u64) }
    }
}
fn deliver(note: &Note) {
    let (kind, target, instance) = note.callback;
    match kind {
        CALLBACK_FUNCTION if target != 0 => {
            let call: WaveProc =
                unsafe { core::mem::transmute::<usize, WaveProc>(target as usize) };
            call(
                note.device as usize,
                note.message,
                instance as usize,
                note.header as usize,
                0,
            );
        }
        CALLBACK_WINDOW if target != 0 => {
            let post = user32(c"PostMessageW");
            if post != 0 {
                let post: PostMessage =
                    unsafe { core::mem::transmute::<usize, PostMessage>(post as usize) };
                post(
                    target as usize,
                    note.message,
                    note.device as usize,
                    note.header as isize,
                );
            }
        }
        CALLBACK_THREAD if target != 0 => {
            let post = user32(c"PostThreadMessageW");
            if post != 0 {
                let post: PostThreadMessage =
                    unsafe { core::mem::transmute::<usize, PostThreadMessage>(post as usize) };
                post(
                    target as u32,
                    note.message,
                    note.device as usize,
                    note.header as isize,
                );
            }
        }
        CALLBACK_EVENT if target != 0 => {
            unsafe { DoorSetEvent(target) };
        }
        _ => (),
    }
}
extern "system" fn worker(_argument: usize) -> u32 {
    {
        loop {
            let mut notes = Vec::new();
            let mut finished_sounds = Vec::new();
            let streams: Vec<(u64, u32)> =
                DEVICES.with(|d| d.iter().map(|d| (d.handle, d.stream)).collect());
            for (handle, stream) in streams {
                let state = played(stream);
                DEVICES.with(|devices| {
                    let Some(d) = devices.iter_mut().find(|d| d.handle == handle) else {
                        return;
                    };
                    if state.is_none() && !d.lost {
                        lose(d, &mut notes);
                    }
                    if let Some((frames, drained)) = state {
                        retire(d, frames, &mut notes);
                        feed(d);
                        if drained && d.pending.is_empty() && d.playing.is_empty() {
                            match d.sound.clone() {
                                Some(SoundMode::Once) => finished_sounds.push(handle),
                                Some(SoundMode::Loop(bytes)) => {
                                    let length = bytes.len();
                                    d.pending.push_back(Buffer::new(
                                        Data::Sound,
                                        Source::Owned(bytes),
                                        length,
                                        1,
                                    ));
                                }
                                None => (),
                            }
                        }
                    }
                });
            }
            for handle in finished_sounds {
                close_device(handle);
            }
            notes.extend(capture_pass());
            notes.extend(midi::pass());
            notes.extend(mci::pass());
            for note in &notes {
                deliver(note);
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
/// Opens a door.audio stream for `wave`.
fn open_stream(wave: &Wave) -> Result<u32, u32> {
    let format = Format {
        rate: wave.rate,
        channels: wave.channels as u8,
        bits: wave.stream_bits(),
    };
    match audio(|a| a.open(format)) {
        Some(Ok(id)) => {
            let app = APP_VOLUME.load(Ordering::Relaxed);
            if app != 0xFFFF_FFFF {
                audio(|a| a.stream_volume(id, percent(app) as u8));
            }
            Ok(id)
        }
        Some(Err(_)) => Err(mm::ALLOCATED),
        None => Err(mm::NODRIVER),
    }
}
fn close_device(handle: u64) -> Option<Device> {
    let device = DEVICES.with(|d| {
        d.iter()
            .position(|x| x.handle == handle)
            .map(|i| d.remove(i))
    })?;
    audio(|a| a.close(device.stream));
    Some(device)
}

// ---- waveOut ------------------------------------------------------------------
#[unsafe(no_mangle)]
pub extern "system" fn waveOutGetNumDevs() -> u32 {
    u32::from(device_present())
}
/// WAVEOUTCAPS: wMid, wPid, vDriverVersion, szPname[32], dwFormats,
/// wChannels, wReserved1, dwSupport. IDs, versions, formats and channels
/// are what Windows reports for a speaker and its sound mapper; dwSupport
/// claims only volume (and sample accuracy for the device): door.audio has
/// no pitch or playback-rate control and one volume per stream.
fn caps(device: u32, wide: bool) -> Result<Vec<u8>, u32> {
    if !device_present() || (device != 0 && device != WAVE_MAPPER) {
        return Err(mm::BADDEVICEID);
    }
    let (pid, version, name, support) = if device == WAVE_MAPPER {
        (2u16, 0x0500u32, "Macrohard Sound Mapper", 0x04u32)
    } else {
        (65535, 0, "Speakers (VirtIO Sound)", 0x24)
    };
    let mut b = Vec::new();
    b.extend_from_slice(&1u16.to_le_bytes());
    b.extend_from_slice(&pid.to_le_bytes());
    b.extend_from_slice(&version.to_le_bytes());
    let mut text = [0u8; 64];
    if wide {
        for (i, u) in name.encode_utf16().take(31).enumerate() {
            text[i * 2..i * 2 + 2].copy_from_slice(&u.to_le_bytes());
        }
        b.extend_from_slice(&text);
    } else {
        let n = name.len().min(31);
        text[..n].copy_from_slice(&name.as_bytes()[..n]);
        b.extend_from_slice(&text[..32]);
    }
    b.extend_from_slice(&0x000F_FFFFu32.to_le_bytes());
    b.extend_from_slice(&2u16.to_le_bytes());
    b.extend_from_slice(&0u16.to_le_bytes());
    b.extend_from_slice(&support.to_le_bytes());
    Ok(b)
}
fn get_caps(device: u64, out: u64, size: u32, wide: bool) -> u32 {
    let device = if device == u64::MAX || device == u64::from(u32::MAX) {
        WAVE_MAPPER
    } else {
        device as u32
    };
    match caps(device, wide) {
        Ok(b) => {
            let n = (size as usize).min(b.len());
            write(out, &b[..n]).err().unwrap_or(mm::NOERROR)
        }
        Err(e) => e,
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn waveOutGetDevCapsW(device: usize, out: usize, size: u32) -> u32 {
    let device = door_windows_abi::widen(device);
    let out = door_windows_abi::widen(out);
    get_caps(device, out, size, true)
}
#[unsafe(no_mangle)]
pub extern "system" fn waveOutGetDevCapsA(device: usize, out: usize, size: u32) -> u32 {
    let device = door_windows_abi::widen(device);
    let out = door_windows_abi::widen(out);
    get_caps(device, out, size, false)
}
#[unsafe(no_mangle)]
pub extern "system" fn waveOutOpen(
    out: usize,
    device: u32,
    format: usize,
    callback: usize,
    instance: usize,
    flags: u32,
) -> u32 {
    let out = door_windows_abi::widen(out);
    let format = door_windows_abi::widen(format);
    let callback = door_windows_abi::widen(callback);
    let instance = door_windows_abi::widen(instance);
    {
        let result = (|| {
            if device != 0 && device != WAVE_MAPPER {
                return Err(mm::BADDEVICEID);
            }
            let mut header = [0u8; 18];
            read(format, &mut header)?;
            let extra = u16::from_le_bytes([header[16], header[17]]) as usize;
            let tag = u16::from_le_bytes([header[0], header[1]]);
            let mut whole = vec![
                0u8;
                18 + if tag == crate::WAVE_FORMAT_EXTENSIBLE {
                    extra.min(22)
                } else {
                    0
                }
            ];
            read(format, &mut whole)?;
            let wave = Wave::parse(&whole).ok_or(mm::BADFORMAT)?;
            if !device_present() {
                return Err(mm::NODRIVER);
            }
            if flags & WAVE_FORMAT_QUERY != 0 {
                return Ok(None);
            }
            let kind = flags & CALLBACK_TYPEMASK;
            if !matches!(
                kind,
                CALLBACK_NULL
                    | CALLBACK_WINDOW
                    | CALLBACK_THREAD
                    | CALLBACK_FUNCTION
                    | CALLBACK_EVENT
            ) {
                return Err(mm::INVALFLAG);
            }
            if out == 0 {
                return Err(mm::INVALPARAM);
            }
            let stream = open_stream(&wave)?;
            let handle = NEXT_HANDLE.fetch_add(0x10, Ordering::Relaxed);
            if write_pointer(out, handle).is_err() {
                audio(|a| a.close(stream));
                return Err(mm::INVALPARAM);
            }
            DEVICES.with(|d| {
                d.push(Device {
                    handle,
                    wave,
                    stream,
                    callback: (kind, callback, instance),
                    pending: VecDeque::new(),
                    playing: VecDeque::new(),
                    sections: VecDeque::new(),
                    submitted: 0,
                    paused: false,
                    volume: 0xFFFF_FFFF,
                    sound: None,
                    lost: false,
                })
            });
            start_worker();
            Ok(Some((handle, (kind, callback, instance))))
        })();
        match result {
            Ok(Some((handle, callback))) => {
                deliver(&Note {
                    device: handle,
                    callback,
                    message: WOM_OPEN,
                    header: 0,
                });
                mm::NOERROR
            }
            Ok(None) => mm::NOERROR,
            Err(e) => e,
        }
    }
}
fn with_device<R>(handle: u64, f: impl FnOnce(&mut Device) -> Result<R, u32>) -> Result<R, u32> {
    DEVICES.with(|d| match d.iter_mut().find(|x| x.handle == handle) {
        Some(device) => f(device),
        None => Err(mm::INVALHANDLE),
    })
}
fn exists(handle: u64) -> bool {
    DEVICES.with(|d| d.iter().any(|x| x.handle == handle))
}
#[unsafe(no_mangle)]
pub extern "system" fn waveOutPrepareHeader(handle: usize, header: usize, size: u32) -> u32 {
    let handle = door_windows_abi::widen(handle);
    let header = door_windows_abi::widen(header);
    {
        let result = (|| {
            if !exists(handle) {
                return Err(mm::INVALHANDLE);
            }
            if size < WAVEHDR_BYTES || header == 0 {
                return Err(mm::INVALPARAM);
            }
            let flags = read_u32(header + WH_FLAGS)?;
            if flags & WHDR_INQUEUE != 0 {
                return Err(mm::STILLPLAYING);
            }
            write_u32(header + WH_FLAGS, flags | WHDR_PREPARED)
        })();
        result.err().unwrap_or(mm::NOERROR)
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn waveOutUnprepareHeader(handle: usize, header: usize, size: u32) -> u32 {
    let handle = door_windows_abi::widen(handle);
    let header = door_windows_abi::widen(header);
    {
        let result = (|| {
            if !exists(handle) {
                return Err(mm::INVALHANDLE);
            }
            if size < WAVEHDR_BYTES || header == 0 {
                return Err(mm::INVALPARAM);
            }
            let flags = read_u32(header + WH_FLAGS)?;
            if flags & WHDR_INQUEUE != 0 {
                return Err(mm::STILLPLAYING);
            }
            write_u32(header + WH_FLAGS, flags & !WHDR_PREPARED)
        })();
        result.err().unwrap_or(mm::NOERROR)
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn waveOutWrite(handle: usize, header: usize, size: u32) -> u32 {
    let handle = door_windows_abi::widen(handle);
    let header = door_windows_abi::widen(header);
    {
        let result = (|| {
            if size < WAVEHDR_BYTES || header == 0 {
                return Err(mm::INVALPARAM);
            }
            let (wave, lost) = with_device(handle, |d| Ok((d.wave, d.lost)))?;
            if lost {
                return Err(mm::NODRIVER);
            }
            let flags = read_u32(header + WH_FLAGS)?;
            if flags & WHDR_PREPARED == 0 {
                return Err(mm::UNPREPARED);
            }
            if flags & WHDR_INQUEUE != 0 {
                return Err(mm::STILLPLAYING);
            }
            let data = read_pointer(header)?;
            let length = read_u32(header + WH_LENGTH)?;
            let loops = if flags & (WHDR_BEGINLOOP | WHDR_ENDLOOP) == WHDR_BEGINLOOP | WHDR_ENDLOOP
            {
                read_u32(header + WH_LOOPS)?.max(1)
            } else {
                1
            };
            // The samples are read when handed to door.audio; here only that
            // the whole buffer is readable.
            let frames = length as usize / usize::from(wave.block_align);
            if frames != 0 {
                let mut probe = [0u8; 1];
                read(data, &mut probe)?;
                read(data + u64::from(length) - 1, &mut probe)?;
            }
            write_u32(header + WH_FLAGS, (flags | WHDR_INQUEUE) & !WHDR_DONE)?;
            with_device(handle, |d| {
                d.pending.push_back(Buffer::new(
                    Data::Header(header),
                    Source::Application { data },
                    frames * wave.stream_frame(),
                    loops,
                ));
                feed(d);
                Ok(())
            })
        })();
        result.err().unwrap_or(mm::NOERROR)
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn waveOutPause(handle: usize) -> u32 {
    let handle = door_windows_abi::widen(handle);
    {
        with_device(handle, |d| {
            d.paused = true;
            audio(|a| a.pause(d.stream, true));
            Ok(())
        })
        .err()
        .unwrap_or(mm::NOERROR)
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn waveOutRestart(handle: usize) -> u32 {
    let handle = door_windows_abi::widen(handle);
    {
        with_device(handle, |d| {
            d.paused = false;
            audio(|a| a.pause(d.stream, false));
            Ok(())
        })
        .err()
        .unwrap_or(mm::NOERROR)
    }
}
/// Stops playback at once: every queued buffer is marked done (and
/// reported), the position returns to 0.
#[unsafe(no_mangle)]
pub extern "system" fn waveOutReset(handle: usize) -> u32 {
    let handle = door_windows_abi::widen(handle);
    {
        let result = with_device(handle, |d| {
            audio(|a| a.close(d.stream));
            let reopened = open_stream(&d.wave);
            let mut notes = Vec::new();
            let pending = core::mem::take(&mut d.pending).into_iter().map(|b| b.data);
            let playing = core::mem::take(&mut d.playing)
                .into_iter()
                .map(|(data, _)| data);
            for data in playing.chain(pending) {
                if let Data::Header(header) = data {
                    if let Ok(flags) = read_u32(header + WH_FLAGS) {
                        let _ = write_u32(header + WH_FLAGS, (flags | WHDR_DONE) & !WHDR_INQUEUE);
                    }
                    notes.push(Note {
                        device: d.handle,
                        callback: d.callback,
                        message: WOM_DONE,
                        header,
                    });
                }
            }
            d.sections.clear();
            d.submitted = 0;
            d.stream = reopened?;
            apply_volume(d);
            if d.paused {
                audio(|a| a.pause(d.stream, true));
            }
            Ok(notes)
        });
        match result {
            // Windows reports the returned buffers before waveOutReset returns.
            Ok(notes) => {
                notes.iter().for_each(deliver);
                mm::NOERROR
            }
            Err(e) => e,
        }
    }
}
/// Ends a WAVEHDR loop: the repetition playing finishes, then playback
/// goes on with the next buffer.
#[unsafe(no_mangle)]
pub extern "system" fn waveOutBreakLoop(handle: usize) -> u32 {
    let handle = door_windows_abi::widen(handle);
    {
        with_device(handle, |d| {
            let submitted = d.submitted;
            if let Some(b) = d.pending.front_mut().filter(|b| b.loops > 1 || b.started) {
                if b.started {
                    let b = d.pending.pop_front().unwrap();
                    d.playing.push_back((b.data, submitted));
                } else {
                    b.loops = 1;
                }
            }
            Ok(())
        })
        .err()
        .unwrap_or(mm::NOERROR)
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn waveOutClose(handle: usize) -> u32 {
    let handle = door_windows_abi::widen(handle);
    {
        let busy = with_device(handle, |d| {
            Ok(!d.pending.is_empty() || !d.playing.is_empty())
        });
        match busy {
            Err(e) => e,
            Ok(true) => mm::STILLPLAYING,
            Ok(false) => match close_device(handle) {
                Some(d) => {
                    deliver(&Note {
                        device: handle,
                        callback: d.callback,
                        message: WOM_CLOSE,
                        header: 0,
                    });
                    mm::NOERROR
                }
                None => mm::INVALHANDLE,
            },
        }
    }
}
/// MMTIME { wType, value }.
#[unsafe(no_mangle)]
pub extern "system" fn waveOutGetPosition(handle: usize, time: usize, size: u32) -> u32 {
    let handle = door_windows_abi::widen(handle);
    let time = door_windows_abi::widen(time);
    {
        let result = (|| {
            if size < 12 {
                return Err(mm::INVALPARAM);
            }
            let (stream, wave) = with_device(handle, |d| Ok((d.stream, d.wave)))?;
            let frames = played(stream).map_or(0, |(f, _)| f);
            let kind = read_u32(time)?;
            let (kind, value) = crate::position(frames, &wave, kind);
            let mut b = [0u8; 12];
            b[..4].copy_from_slice(&kind.to_le_bytes());
            b[4..8].copy_from_slice(&value.to_le_bytes());
            write(time, &b)
        })();
        result.err().unwrap_or(mm::NOERROR)
    }
}
/// The application's volume, set through a device ID rather than a handle
/// (on Windows the process's audio session volume).
static APP_VOLUME: AtomicU32 = AtomicU32::new(0xFFFF_FFFF);
/// Low word left, high word right (0..=0xFFFF); door.audio streams have
/// one volume, the average, in percent.
fn percent(volume: u32) -> u32 {
    ((volume & 0xFFFF) + (volume >> 16)) / 2 * 100 / 0xFFFF
}
fn apply_volume(d: &Device) {
    let level = percent(d.volume) * percent(APP_VOLUME.load(Ordering::Relaxed)) / 100;
    audio(|a| a.stream_volume(d.stream, level as u8));
}
fn is_device_id(handle: u64) -> bool {
    handle == 0 || handle == u64::from(WAVE_MAPPER) || handle == u64::MAX
}
#[unsafe(no_mangle)]
pub extern "system" fn waveOutSetVolume(handle: usize, volume: u32) -> u32 {
    let handle = door_windows_abi::widen(handle);
    {
        if is_device_id(handle) {
            if !device_present() {
                return mm::NODRIVER;
            }
            APP_VOLUME.store(volume, Ordering::Relaxed);
            DEVICES.with(|devices| devices.iter().for_each(apply_volume));
            return mm::NOERROR;
        }
        with_device(handle, |d| {
            d.volume = volume;
            apply_volume(d);
            Ok(())
        })
        .err()
        .unwrap_or(mm::NOERROR)
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn waveOutGetVolume(handle: usize, out: usize) -> u32 {
    let handle = door_windows_abi::widen(handle);
    let out = door_windows_abi::widen(out);
    {
        let volume = if is_device_id(handle) {
            if device_present() {
                Ok(APP_VOLUME.load(Ordering::Relaxed))
            } else {
                Err(mm::NODRIVER)
            }
        } else {
            with_device(handle, |d| Ok(d.volume))
        };
        volume
            .and_then(|v| write_u32(out, v))
            .err()
            .unwrap_or(mm::NOERROR)
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn waveOutGetID(handle: usize, out: usize) -> u32 {
    let handle = door_windows_abi::widen(handle);
    let out = door_windows_abi::widen(out);
    {
        let result = with_device(handle, |_| Ok(())).and_then(|()| write_u32(out, 0));
        result.err().unwrap_or(mm::NOERROR)
    }
}
fn error_text(code: u32, out: u64, length: u32, wide: bool) -> u32 {
    let Some(text) = crate::error_text(code) else {
        return mm::BADERRNUM;
    };
    if out == 0 || length == 0 {
        return mm::INVALPARAM;
    }
    let bytes: Vec<u8> = if wide {
        text.encode_utf16()
            .take(length as usize - 1)
            .chain([0])
            .flat_map(u16::to_le_bytes)
            .collect()
    } else {
        text.bytes().take(length as usize - 1).chain([0]).collect()
    };
    write(out, &bytes).err().unwrap_or(mm::NOERROR)
}
#[unsafe(no_mangle)]
pub extern "system" fn waveOutGetErrorTextW(code: u32, out: usize, length: u32) -> u32 {
    let out = door_windows_abi::widen(out);
    error_text(code, out, length, true)
}
#[unsafe(no_mangle)]
pub extern "system" fn waveOutGetErrorTextA(code: u32, out: usize, length: u32) -> u32 {
    let out = door_windows_abi::widen(out);
    error_text(code, out, length, false)
}
#[unsafe(no_mangle)]
pub extern "system" fn waveOutMessage(_handle: usize, _message: u32, _a: usize, _b: usize) -> u32 {
    mm::NOTSUPPORTED
}
#[unsafe(no_mangle)]
pub extern "system" fn waveOutGetPitch(_handle: usize, _out: usize) -> u32 {
    mm::NOTSUPPORTED
}
#[unsafe(no_mangle)]
pub extern "system" fn waveOutSetPitch(_handle: usize, _value: u32) -> u32 {
    mm::NOTSUPPORTED
}
#[unsafe(no_mangle)]
pub extern "system" fn waveOutGetPlaybackRate(_handle: usize, _out: usize) -> u32 {
    mm::NOTSUPPORTED
}
#[unsafe(no_mangle)]
pub extern "system" fn waveOutSetPlaybackRate(_handle: usize, _value: u32) -> u32 {
    mm::NOTSUPPORTED
}

// ---- waveIn -----------------------------------------------------------------------------
// Each open device is a door.audio capture stream filling a ring in a
// section this process maps for good; the worker converts what the ring
// holds into the application's buffers (WIM_DATA when one is full).
use door_service_protocol::audio::capture as ring;

const WIM_OPEN: u32 = 0x3BE;
const WIM_CLOSE: u32 = 0x3BF;
const WIM_DATA: u32 = 0x3C0;
/// Ring capacity: two seconds of captured sound.
const RING_BYTES: usize = 200 * ring::PERIOD_BYTES;
/// Converted sound kept while no buffer waits (then dropped, as a device
/// with an empty queue does): one second at most.
const LEFTOVER_LIMIT: usize = 192_000;
struct InDevice {
    handle: u64,
    wave: Wave,
    callback: (u32, u64, u64),
    capture: u32,
    section: u64,
    /// The ring's mapping in this process (header, then data).
    ring: u64,
    mapped: u64,
    /// Queued application WAVEHDRs, oldest first.
    queue: VecDeque<u64>,
    started: bool,
    recorder: crate::Recorder,
    /// Converted bytes no buffer had room for yet.
    leftover: Vec<u8>,
    /// Bytes delivered into buffers since the last reset (the position).
    recorded: u64,
    /// The ring stopped (policy or device): nothing more comes.
    ended: bool,
}
static IN_DEVICES: Lock<Vec<InDevice>> = Lock::new(Vec::new());
fn ring_word(base: u64, at: usize) -> &'static AtomicU32 {
    // SAFETY: the ring stays mapped while its device exists; fields aligned.
    unsafe { &*((base + at as u64) as *const AtomicU32) }
}
fn ring_count(base: u64, at: usize) -> &'static AtomicU64 {
    // SAFETY: as ring_word.
    unsafe { &*((base + at as u64) as *const AtomicU64) }
}
/// The process executable file name (the Microphone page lists it).
fn executable_name() -> String {
    let mut units = [0u16; 260];
    let n = unsafe { DoorGetModuleFileNameW(0, units.as_mut_ptr() as u64, units.len() as u32) }
        as usize;
    let path = String::from_utf16_lossy(&units[..n.min(units.len())]);
    let name = path.rsplit(['\\', '/']).next().unwrap_or("");
    let mut end = name.len().min(door_service_protocol::audio::NAME_BYTES);
    while !name.is_char_boundary(end) {
        end -= 1;
    }
    String::from(&name[..end])
}
/// Capture input present (door.audio status).
fn capture_present() -> bool {
    audio(|a| a.status().is_ok_and(|s| s.present && s.capture)).unwrap_or(false)
}
#[unsafe(no_mangle)]
pub extern "system" fn waveInGetNumDevs() -> u32 {
    u32::from(capture_present())
}
/// WAVEINCAPS: wMid, wPid, vDriverVersion, szPname[32], dwFormats,
/// wChannels, wReserved1, as Windows reports a microphone and its mapper.
fn in_caps(device: u64, out: u64, size: u32, wide: bool) -> u32 {
    let device = if device == u64::MAX || device == u64::from(u32::MAX) {
        WAVE_MAPPER
    } else {
        device as u32
    };
    if !capture_present() || (device != 0 && device != WAVE_MAPPER) {
        return mm::BADDEVICEID;
    }
    let (pid, version, name) = if device == WAVE_MAPPER {
        (2u16, 0x0500u32, "Macrohard Sound Mapper")
    } else {
        (65535, 0, "Microphone (VirtIO Sound)")
    };
    let mut b = Vec::new();
    b.extend_from_slice(&1u16.to_le_bytes());
    b.extend_from_slice(&pid.to_le_bytes());
    b.extend_from_slice(&version.to_le_bytes());
    let mut text = [0u8; 64];
    if wide {
        for (i, u) in name.encode_utf16().take(31).enumerate() {
            text[i * 2..i * 2 + 2].copy_from_slice(&u.to_le_bytes());
        }
        b.extend_from_slice(&text);
    } else {
        let n = name.len().min(31);
        text[..n].copy_from_slice(&name.as_bytes()[..n]);
        b.extend_from_slice(&text[..32]);
    }
    b.extend_from_slice(&0x000F_FFFFu32.to_le_bytes());
    b.extend_from_slice(&2u16.to_le_bytes());
    b.extend_from_slice(&0u16.to_le_bytes());
    let n = (size as usize).min(b.len());
    write(out, &b[..n]).err().unwrap_or(mm::NOERROR)
}
#[unsafe(no_mangle)]
pub extern "system" fn waveInGetDevCapsW(device: usize, out: usize, size: u32) -> u32 {
    let device = door_windows_abi::widen(device);
    let out = door_windows_abi::widen(out);
    in_caps(device, out, size, true)
}
#[unsafe(no_mangle)]
pub extern "system" fn waveInGetDevCapsA(device: usize, out: usize, size: u32) -> u32 {
    let device = door_windows_abi::widen(device);
    let out = door_windows_abi::widen(out);
    in_caps(device, out, size, false)
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
    // SAFETY: no reference into the ring outlives its device.
    if unsafe { door_sys::section_unmap(address, mapped) }.is_ok() {
        unsafe { DoorReleaseSectionRange(address, mapped) };
    }
    let _ = door_sys::close(section);
}
#[unsafe(no_mangle)]
pub extern "system" fn waveInOpen(
    out: usize,
    device: u32,
    format: usize,
    callback: usize,
    instance: usize,
    flags: u32,
) -> u32 {
    let out = door_windows_abi::widen(out);
    let format = door_windows_abi::widen(format);
    let callback = door_windows_abi::widen(callback);
    let instance = door_windows_abi::widen(instance);
    {
        let result = (|| {
            if device != 0 && device != WAVE_MAPPER {
                return Err(mm::BADDEVICEID);
            }
            let mut header = [0u8; 18];
            read(format, &mut header)?;
            let tag = u16::from_le_bytes([header[0], header[1]]);
            let extra = u16::from_le_bytes([header[16], header[17]]) as usize;
            let mut whole = vec![
                0u8;
                18 + if tag == crate::WAVE_FORMAT_EXTENSIBLE {
                    extra.min(22)
                } else {
                    0
                }
            ];
            read(format, &mut whole)?;
            let wave = Wave::parse(&whole).ok_or(mm::BADFORMAT)?;
            if !capture_present() {
                return Err(mm::BADDEVICEID);
            }
            if flags & WAVE_FORMAT_QUERY != 0 {
                return Ok(None);
            }
            let kind = flags & CALLBACK_TYPEMASK;
            if !matches!(
                kind,
                CALLBACK_NULL
                    | CALLBACK_WINDOW
                    | CALLBACK_THREAD
                    | CALLBACK_FUNCTION
                    | CALLBACK_EVENT
            ) {
                return Err(mm::INVALFLAG);
            }
            if out == 0 {
                return Err(mm::INVALPARAM);
            }
            let (section, address, mapped) = create_ring().ok_or(mm::NOMEM)?;
            // Refused by Privacy & security > Microphone: MMSYSERR_ERROR, the
            // result applications report as "Wav Error: 1" on Windows.
            let capture = match audio(|a| a.capture_open(section, &executable_name())) {
                Some(Ok(id)) => id,
                Some(Err(door_service_client::Error::Service(
                    door_service_protocol::Code::Denied,
                ))) => {
                    release_ring(section, address, mapped);
                    return Err(mm::ERROR);
                }
                Some(Err(_)) => {
                    release_ring(section, address, mapped);
                    return Err(mm::ALLOCATED);
                }
                None => {
                    release_ring(section, address, mapped);
                    return Err(mm::NODRIVER);
                }
            };
            let handle = NEXT_HANDLE.fetch_add(0x10, Ordering::Relaxed);
            if write_pointer(out, handle).is_err() {
                audio(|a| a.capture_close(capture));
                release_ring(section, address, mapped);
                return Err(mm::INVALPARAM);
            }
            IN_DEVICES.with(|d| {
                d.push(InDevice {
                    handle,
                    wave,
                    callback: (kind, callback, instance),
                    capture,
                    section,
                    ring: address,
                    mapped,
                    queue: VecDeque::new(),
                    started: false,
                    recorder: crate::Recorder::new(wave),
                    leftover: Vec::new(),
                    recorded: 0,
                    ended: false,
                })
            });
            start_worker();
            Ok(Some((handle, (kind, callback, instance))))
        })();
        match result {
            Ok(Some((handle, callback))) => {
                deliver(&Note {
                    device: handle,
                    callback,
                    message: WIM_OPEN,
                    header: 0,
                });
                mm::NOERROR
            }
            Ok(None) => mm::NOERROR,
            Err(e) => e,
        }
    }
}
fn with_in<R>(handle: u64, f: impl FnOnce(&mut InDevice) -> Result<R, u32>) -> Result<R, u32> {
    IN_DEVICES.with(|d| match d.iter_mut().find(|x| x.handle == handle) {
        Some(device) => f(device),
        None => Err(mm::INVALHANDLE),
    })
}
fn in_header(handle: u64, header: u64, size: u32, set: bool) -> u32 {
    let result = (|| {
        with_in(handle, |_| Ok(()))?;
        if size < WAVEHDR_BYTES || header == 0 {
            return Err(mm::INVALPARAM);
        }
        let flags = read_u32(header + WH_FLAGS)?;
        if flags & WHDR_INQUEUE != 0 {
            return Err(mm::STILLPLAYING);
        }
        write_u32(
            header + WH_FLAGS,
            if set {
                flags | WHDR_PREPARED
            } else {
                flags & !WHDR_PREPARED
            },
        )
    })();
    result.err().unwrap_or(mm::NOERROR)
}
#[unsafe(no_mangle)]
pub extern "system" fn waveInPrepareHeader(handle: usize, header: usize, size: u32) -> u32 {
    let handle = door_windows_abi::widen(handle);
    let header = door_windows_abi::widen(header);
    in_header(handle, header, size, true)
}
#[unsafe(no_mangle)]
pub extern "system" fn waveInUnprepareHeader(handle: usize, header: usize, size: u32) -> u32 {
    let handle = door_windows_abi::widen(handle);
    let header = door_windows_abi::widen(header);
    in_header(handle, header, size, false)
}
#[unsafe(no_mangle)]
pub extern "system" fn waveInAddBuffer(handle: usize, header: usize, size: u32) -> u32 {
    let handle = door_windows_abi::widen(handle);
    let header = door_windows_abi::widen(header);
    {
        let result = (|| {
            if size < WAVEHDR_BYTES || header == 0 {
                return Err(mm::INVALPARAM);
            }
            with_in(handle, |_| Ok(()))?;
            let flags = read_u32(header + WH_FLAGS)?;
            if flags & WHDR_PREPARED == 0 {
                return Err(mm::UNPREPARED);
            }
            if flags & WHDR_INQUEUE != 0 {
                return Err(mm::STILLPLAYING);
            }
            write_u32(header + WH_RECORDED, 0)?;
            write_u32(header + WH_FLAGS, (flags | WHDR_INQUEUE) & !WHDR_DONE)?;
            with_in(handle, |d| {
                d.queue.push_back(header);
                Ok(())
            })
        })();
        result.err().unwrap_or(mm::NOERROR)
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn waveInStart(handle: usize) -> u32 {
    let handle = door_windows_abi::widen(handle);
    {
        with_in(handle, |d| {
            if !d.started {
                // Sound captured before the start is not recorded.
                let written = ring_count(d.ring, ring::WRITE).load(Ordering::Acquire);
                ring_count(d.ring, ring::READ).store(written, Ordering::Release);
                d.started = true;
            }
            Ok(())
        })
        .err()
        .unwrap_or(mm::NOERROR)
    }
}
/// Returns the current buffer (partly filled) as done; empty buffers stay
/// queued.
fn return_partial(d: &mut InDevice, notes: &mut Vec<Note>) {
    if let Some(&header) = d.queue.front()
        && read_u32(header + WH_RECORDED).is_ok_and(|n| n > 0)
    {
        d.queue.pop_front();
        if let Ok(flags) = read_u32(header + WH_FLAGS) {
            let _ = write_u32(header + WH_FLAGS, (flags | WHDR_DONE) & !WHDR_INQUEUE);
        }
        notes.push(Note {
            device: d.handle,
            callback: d.callback,
            message: WIM_DATA,
            header,
        });
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn waveInStop(handle: usize) -> u32 {
    let handle = door_windows_abi::widen(handle);
    {
        let mut notes = Vec::new();
        let result = with_in(handle, |d| {
            d.started = false;
            d.leftover.clear();
            return_partial(d, &mut notes);
            Ok(())
        });
        notes.iter().for_each(deliver);
        result.err().unwrap_or(mm::NOERROR)
    }
}
/// Stops recording and returns every queued buffer; the position restarts.
#[unsafe(no_mangle)]
pub extern "system" fn waveInReset(handle: usize) -> u32 {
    let handle = door_windows_abi::widen(handle);
    {
        let mut notes = Vec::new();
        let result = with_in(handle, |d| {
            d.started = false;
            d.leftover.clear();
            d.recorded = 0;
            d.recorder = crate::Recorder::new(d.wave);
            for header in core::mem::take(&mut d.queue) {
                if let Ok(flags) = read_u32(header + WH_FLAGS) {
                    let _ = write_u32(header + WH_FLAGS, (flags | WHDR_DONE) & !WHDR_INQUEUE);
                }
                notes.push(Note {
                    device: d.handle,
                    callback: d.callback,
                    message: WIM_DATA,
                    header,
                });
            }
            Ok(())
        });
        notes.iter().for_each(deliver);
        result.err().unwrap_or(mm::NOERROR)
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn waveInClose(handle: usize) -> u32 {
    let handle = door_windows_abi::widen(handle);
    {
        let busy = with_in(handle, |d| Ok(!d.queue.is_empty()));
        match busy {
            Err(e) => e,
            Ok(true) => mm::STILLPLAYING,
            Ok(false) => {
                let Some(d) = IN_DEVICES.with(|d| {
                    d.iter()
                        .position(|x| x.handle == handle)
                        .map(|i| d.remove(i))
                }) else {
                    return mm::INVALHANDLE;
                };
                audio(|a| a.capture_close(d.capture));
                release_ring(d.section, d.ring, d.mapped);
                deliver(&Note {
                    device: handle,
                    callback: d.callback,
                    message: WIM_CLOSE,
                    header: 0,
                });
                mm::NOERROR
            }
        }
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn waveInGetPosition(handle: usize, time: usize, size: u32) -> u32 {
    let handle = door_windows_abi::widen(handle);
    let time = door_windows_abi::widen(time);
    {
        let result = (|| {
            if size < 12 {
                return Err(mm::INVALPARAM);
            }
            let (recorded, wave) = with_in(handle, |d| Ok((d.recorded, d.wave)))?;
            let frames = recorded / u64::from(wave.block_align);
            let kind = read_u32(time)?;
            let (kind, value) = crate::position(frames, &wave, kind);
            let mut b = [0u8; 12];
            b[..4].copy_from_slice(&kind.to_le_bytes());
            b[4..8].copy_from_slice(&value.to_le_bytes());
            write(time, &b)
        })();
        result.err().unwrap_or(mm::NOERROR)
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn waveInGetID(handle: usize, out: usize) -> u32 {
    let handle = door_windows_abi::widen(handle);
    let out = door_windows_abi::widen(out);
    {
        let result = with_in(handle, |_| Ok(())).and_then(|()| write_u32(out, 0));
        result.err().unwrap_or(mm::NOERROR)
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn waveInGetErrorTextW(code: u32, out: usize, length: u32) -> u32 {
    let out = door_windows_abi::widen(out);
    error_text(code, out, length, true)
}
#[unsafe(no_mangle)]
pub extern "system" fn waveInGetErrorTextA(code: u32, out: usize, length: u32) -> u32 {
    let out = door_windows_abi::widen(out);
    error_text(code, out, length, false)
}
#[unsafe(no_mangle)]
pub extern "system" fn waveInMessage(_handle: usize, _message: u32, _a: usize, _b: usize) -> u32 {
    mm::NOTSUPPORTED
}
/// One worker pass over the capture devices: move what each ring holds into
/// the application's buffers; the WIM_DATA notes to deliver.
fn capture_pass() -> Vec<Note> {
    let mut notes = Vec::new();
    IN_DEVICES.with(|devices| {
        for d in devices.iter_mut() {
            let written = ring_count(d.ring, ring::WRITE).load(Ordering::Acquire);
            let mut read = ring_count(d.ring, ring::READ).load(Ordering::Relaxed);
            if !d.ended && ring_word(d.ring, ring::STATE).load(Ordering::Acquire) != ring::RUNNING {
                // Revoked or lost: what was captured is delivered, then the
                // current buffer returns and nothing more comes.
                d.ended = true;
            }
            if d.started {
                let data = d.ring + ring::HEADER as u64;
                while read < written {
                    let at = (read % RING_BYTES as u64) as usize;
                    let take = ((written - read) as usize)
                        .min(RING_BYTES - at)
                        .min(ring::PERIOD_BYTES * 10);
                    // SAFETY: the producer finished these bytes (Acquire above).
                    let bytes = unsafe {
                        core::slice::from_raw_parts((data + at as u64) as *const u8, take)
                    };
                    d.recorder.push(bytes, &mut d.leftover);
                    read += take as u64;
                }
                if d.leftover.len() > LEFTOVER_LIMIT && d.queue.is_empty() {
                    let excess = d.leftover.len() - LEFTOVER_LIMIT;
                    d.leftover.drain(..excess);
                }
            }
            ring_count(d.ring, ring::READ).store(written, Ordering::Release);
            // Fill buffers in order.
            while let Some(&header) = d.queue.front() {
                if d.leftover.is_empty() {
                    break;
                }
                let (Ok(target), Ok(length), Ok(filled)) = (
                    read_pointer(header),
                    read_u32(header + WH_LENGTH),
                    read_u32(header + WH_RECORDED),
                ) else {
                    d.queue.pop_front();
                    continue;
                };
                let room = (length - filled.min(length)) as usize;
                let n = room.min(d.leftover.len());
                if write(target + u64::from(filled), &d.leftover[..n]).is_err() {
                    d.queue.pop_front();
                    continue;
                }
                d.leftover.drain(..n);
                d.recorded += n as u64;
                let _ = write_u32(header + WH_RECORDED, filled + n as u32);
                if filled + n as u32 >= length {
                    d.queue.pop_front();
                    if let Ok(flags) = read_u32(header + WH_FLAGS) {
                        let _ = write_u32(header + WH_FLAGS, (flags | WHDR_DONE) & !WHDR_INQUEUE);
                    }
                    notes.push(Note {
                        device: d.handle,
                        callback: d.callback,
                        message: WIM_DATA,
                        header,
                    });
                }
            }
            if d.ended && d.started && d.leftover.is_empty() {
                d.started = false;
                return_partial(d, &mut notes);
            }
        }
    });
    notes
}

// ---- MIDI input and auxiliary devices -------------------------------------------------
/// Like a Windows PC without a MIDI keyboard or auxiliary audio hardware
/// (measured on the reference PC: both counts 0, BADDEVICEID).
macro_rules! no_devices {
    ($($name:ident),* $(,)?) => {$(
        #[unsafe(no_mangle)]
        pub extern "system" fn $name() -> u32 {
            0
        }
    )*};
}
no_devices!(midiInGetNumDevs, auxGetNumDevs);
macro_rules! bad_device {
    ($($name:ident($($arg:ident),*)),* $(,)?) => {$(
        #[unsafe(no_mangle)]
        pub extern "system" fn $name($($arg: usize),*) -> u32 {
            $(let _ = $arg;)*
            mm::BADDEVICEID
        }
    )*};
}
/// No MIDI input device can be opened, so no handle names one.
macro_rules! no_handle {
    ($($name:ident($($arg:ident),*)),* $(,)?) => {$(
        #[unsafe(no_mangle)]
        pub extern "system" fn $name($($arg: usize),*) -> u32 {
            $(let _ = $arg;)*
            mm::INVALHANDLE
        }
    )*};
}
no_handle!(
    midiInClose(handle),
    midiInStart(handle),
    midiInStop(handle),
    midiInReset(handle),
    midiInPrepareHeader(handle, header, size),
    midiInUnprepareHeader(handle, header, size),
    midiInAddBuffer(handle, header, size),
    midiInGetID(handle, out),
    midiInMessage(handle, message, a, b),
);
bad_device!(
    midiInGetDevCapsW(device, out, size),
    midiInGetDevCapsA(device, out, size),
    midiInOpen(out, device, callback, instance, flags),
    auxGetDevCapsW(device, out, size),
    auxGetDevCapsA(device, out, size),
    auxGetVolume(device, out),
    auxSetVolume(device, volume),
    auxOutMessage(device, message, a, b),
);

// ---- PlaySound -------------------------------------------------------------------------
const SND_ASYNC: u32 = 0x1;
const SND_NODEFAULT: u32 = 0x2;
const SND_MEMORY: u32 = 0x4;
const SND_LOOP: u32 = 0x8;
const SND_NOSTOP: u32 = 0x10;
const SND_PURGE: u32 = 0x40;
const SND_ALIAS: u32 = 0x0001_0000;
const SND_ALIAS_ID: u32 = 0x0011_0000;
const SND_FILENAME: u32 = 0x0002_0000;
const SND_RESOURCE: u32 = 0x0004_0004;
/// The PlaySound device playing now.
static SOUND: AtomicU64 = AtomicU64::new(0);
fn stop_sound() {
    let current = SOUND.swap(0, Ordering::AcqRel);
    if current != 0 {
        close_device(current);
    }
}
fn text_arg(pointer: u64, wide: bool) -> Result<String, u32> {
    let mut units = Vec::new();
    for i in 0..1024u64 {
        if wide {
            let mut b = [0u8; 2];
            read(pointer + i * 2, &mut b)?;
            let u = u16::from_le_bytes(b);
            if u == 0 {
                break;
            }
            units.push(u);
        } else {
            let mut b = [0u8; 1];
            read(pointer + i, &mut b)?;
            if b[0] == 0 {
                break;
            }
            units.push(u16::from(b[0]));
        }
    }
    Ok(String::from_utf16_lossy(&units))
}
fn read_file(path: &str) -> Option<Vec<u8>> {
    let wide: Vec<u16> = path.encode_utf16().chain([0]).collect();
    // GENERIC_READ, FILE_SHARE_READ, OPEN_EXISTING.
    let file = unsafe { DoorCreateFileW(wide.as_ptr() as u64, 0x8000_0000, 1, 0, 3, 0, 0) };
    if file == u64::MAX || file == 0 {
        return None;
    }
    let mut data = Vec::new();
    let mut chunk = vec![0u8; 64 * 1024];
    loop {
        let mut got = 0u32;
        let ok = unsafe {
            DoorReadFile(
                file,
                chunk.as_mut_ptr() as u64,
                chunk.len() as u32,
                &mut got as *mut u32 as u64,
                0,
            )
        };
        if ok == 0 || got == 0 || data.len() > 64 * 1024 * 1024 {
            break;
        }
        data.extend_from_slice(&chunk[..got as usize]);
    }
    unsafe { DoorCloseFile(file) };
    Some(data)
}
/// Writes `data` as the whole of the file at `path` (created or replaced).
fn write_file(path: &str, data: &[u8]) -> bool {
    let wide: Vec<u16> = path.encode_utf16().chain([0]).collect();
    // GENERIC_WRITE, no sharing, CREATE_ALWAYS.
    let file = unsafe { DoorCreateFileW(wide.as_ptr() as u64, 0x4000_0000, 0, 0, 2, 0, 0) };
    if file == u64::MAX || file == 0 {
        return false;
    }
    let mut ok = true;
    for chunk in data.chunks(64 * 1024) {
        let mut wrote = 0u32;
        let r = unsafe {
            DoorWriteFile(
                file,
                chunk.as_ptr() as u64,
                chunk.len() as u32,
                &mut wrote as *mut u32 as u64,
                0,
            )
        };
        if r == 0 || wrote as usize != chunk.len() {
            ok = false;
            break;
        }
    }
    unsafe { DoorCloseFile(file) };
    ok
}
/// A RIFF image in the caller's memory: its header gives the size.
fn read_memory(pointer: u64) -> Option<Vec<u8>> {
    let mut head = [0u8; 8];
    read(pointer, &mut head).ok()?;
    if &head[..4] != b"RIFF" {
        return None;
    }
    let size = (u32::from_le_bytes(head[4..8].try_into().unwrap()) as usize)
        .saturating_add(8)
        .min(64 * 1024 * 1024);
    let mut image = vec![0u8; size];
    read(pointer, &mut image).ok()?;
    Some(image)
}
fn read_resource(module: u64, name: u64) -> Option<Vec<u8>> {
    let kind: Vec<u16> = "WAVE".encode_utf16().chain([0]).collect();
    let resource = unsafe { DoorFindResource(module, kind.as_ptr() as u64, name, 0) };
    if resource == 0 {
        return None;
    }
    let address = unsafe { DoorLoadResource(module, resource) };
    let size = unsafe { DoorSizeofResource(module, resource) } as usize;
    let mut image = vec![0u8; size];
    read(address, &mut image).ok()?;
    Some(image)
}
/// Plays a WAV image on a PlaySound device; waits for its end unless
/// `asynchronous`.
fn play_image(image: &[u8], asynchronous: bool, looping: bool) -> bool {
    let Some((wave, range)) = crate::parse_wav(image) else {
        return false;
    };
    let bytes = Arc::new(wave.convert(&image[range]));
    let Ok(stream) = open_stream(&wave) else {
        return false;
    };
    let handle = NEXT_HANDLE.fetch_add(0x10, Ordering::Relaxed);
    let mode = if looping {
        SoundMode::Loop(bytes.clone())
    } else {
        SoundMode::Once
    };
    let length = bytes.len();
    DEVICES.with(|d| {
        let mut device = Device {
            handle,
            wave,
            stream,
            callback: (CALLBACK_NULL, 0, 0),
            pending: VecDeque::from([Buffer::new(Data::Sound, Source::Owned(bytes), length, 1)]),
            playing: VecDeque::new(),
            sections: VecDeque::new(),
            submitted: 0,
            paused: false,
            volume: 0xFFFF_FFFF,
            sound: Some(mode),
            lost: false,
        };
        feed(&mut device);
        d.push(device);
    });
    SOUND.store(handle, Ordering::Release);
    start_worker();
    if !asynchronous {
        // A synchronous sound returns when it has played.
        while DEVICES.with(|d| d.iter().any(|x| x.handle == handle)) {
            unsafe { DoorSleep(10) };
        }
    }
    true
}
fn play_alias(name: &str, flags: u32) -> bool {
    match crate::alias(name) {
        Some(Some(event)) => audio(|a| a.play_sound(event).is_ok()).unwrap_or(false),
        // Assigned no sound in the scheme: nothing plays, successfully.
        Some(None) => true,
        // An unknown alias plays the default sound unless SND_NODEFAULT.
        None if flags & SND_NODEFAULT == 0 => audio(|a| {
            a.play_sound(door_service_protocol::audio::sound::DEFAULT_BEEP)
                .is_ok()
        })
        .unwrap_or(false),
        None => false,
    }
}
fn play_sound(sound: u64, module: u64, flags: u32, wide: bool) -> i32 {
    if sound == 0 || flags & SND_PURGE != 0 {
        stop_sound();
        return 1;
    }
    if flags & SND_NOSTOP != 0 && SOUND.load(Ordering::Acquire) != 0 {
        return 0;
    }
    stop_sound();
    let asynchronous = flags & SND_ASYNC != 0;
    let looping = flags & SND_LOOP != 0 && asynchronous;
    let image = if flags & SND_ALIAS_ID == SND_ALIAS_ID {
        return i32::from(crate::alias_id(sound as u32).is_some_and(|n| play_alias(n, flags)));
    } else if flags & SND_MEMORY != 0 && flags & SND_RESOURCE != SND_RESOURCE {
        read_memory(sound)
    } else if flags & SND_RESOURCE == SND_RESOURCE {
        read_resource(module, sound)
    } else {
        let Ok(name) = text_arg(sound, wide) else {
            return 0;
        };
        if flags & SND_ALIAS != 0 {
            return i32::from(play_alias(&name, flags));
        }
        // Without SND_ALIAS or SND_FILENAME, an alias is tried first.
        if flags & SND_FILENAME == 0 && crate::alias(&name).is_some() {
            return i32::from(play_alias(&name, flags));
        }
        match read_file(&name) {
            Some(image) => Some(image),
            None if flags & SND_NODEFAULT == 0 => return i32::from(play_alias(".Default", flags)),
            None => None,
        }
    };
    match image {
        Some(image) => i32::from(play_image(&image, asynchronous, looping)),
        None => 0,
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn PlaySoundW(sound: usize, module: usize, flags: u32) -> i32 {
    let sound = door_windows_abi::widen(sound);
    let module = door_windows_abi::widen(module);
    play_sound(sound, module, flags, true)
}
#[unsafe(no_mangle)]
pub extern "system" fn PlaySoundA(sound: usize, module: usize, flags: u32) -> i32 {
    let sound = door_windows_abi::widen(sound);
    let module = door_windows_abi::widen(module);
    play_sound(sound, module, flags, false)
}
#[unsafe(no_mangle)]
pub extern "system" fn PlaySound(sound: usize, module: usize, flags: u32) -> i32 {
    let sound = door_windows_abi::widen(sound);
    let module = door_windows_abi::widen(module);
    play_sound(sound, module, flags, false)
}
/// sndPlaySound: PlaySound's flags without a module (SND_MEMORY allowed).
#[unsafe(no_mangle)]
pub extern "system" fn sndPlaySoundW(sound: usize, flags: u32) -> i32 {
    let sound = door_windows_abi::widen(sound);
    play_sound(sound, 0, flags & 0xFFFF, true)
}
#[unsafe(no_mangle)]
pub extern "system" fn sndPlaySoundA(sound: usize, flags: u32) -> i32 {
    let sound = door_windows_abi::widen(sound);
    play_sound(sound, 0, flags & 0xFFFF, false)
}

// ---- time -------------------------------------------------------------------------------
const TIMERR_NOERROR: u32 = 0;
const TIMERR_NOCANDO: u32 = 97;
#[unsafe(no_mangle)]
pub extern "system" fn timeGetTime() -> u32 {
    tick() as u32
}
#[unsafe(no_mangle)]
pub extern "system" fn timeGetSystemTime(time: usize, size: u32) -> u32 {
    let time = door_windows_abi::widen(time);
    {
        if size < 12 {
            return mm::INVALPARAM;
        }
        let mut b = [0u8; 12];
        b[..4].copy_from_slice(&crate::TIME_MS.to_le_bytes());
        b[4..8].copy_from_slice(&(tick() as u32).to_le_bytes());
        write(time, &b).err().unwrap_or(mm::NOERROR)
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn timeBeginPeriod(period: u32) -> u32 {
    if (1..=1_000_000).contains(&period) {
        TIMERR_NOERROR
    } else {
        TIMERR_NOCANDO
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn timeEndPeriod(period: u32) -> u32 {
    timeBeginPeriod(period)
}
/// TIMECAPS { wPeriodMin 1, wPeriodMax 1000000 }, as Windows reports.
#[unsafe(no_mangle)]
pub extern "system" fn timeGetDevCaps(caps: usize, size: u32) -> u32 {
    let caps = door_windows_abi::widen(caps);
    {
        if size < 8 {
            return TIMERR_NOCANDO;
        }
        let mut b = [0u8; 8];
        b[..4].copy_from_slice(&1u32.to_le_bytes());
        b[4..].copy_from_slice(&1_000_000u32.to_le_bytes());
        if write(caps, &b).is_err() {
            TIMERR_NOCANDO
        } else {
            TIMERR_NOERROR
        }
    }
}
const TIME_PERIODIC: u32 = 0x1;
const TIME_CALLBACK_EVENT_SET: u32 = 0x10;
const TIME_CALLBACK_EVENT_PULSE: u32 = 0x20;
const TIME_KILL_SYNCHRONOUS: u32 = 0x100;
struct Timer {
    id: u32,
    delay: u32,
    callback: u64,
    user: u64,
    flags: u32,
    live: AtomicBool,
    running: AtomicBool,
}
static TIMERS: Lock<Vec<&'static Timer>> = Lock::new(Vec::new());
static NEXT_TIMER: AtomicU32 = AtomicU32::new(1);
/// LPTIMECALLBACK(uTimerID, uMsg, dwUser, dw1, dw2) at pointer width.
type TimeProc = extern "system" fn(u32, u32, usize, usize, usize);
extern "system" fn timer_thread(argument: usize) -> u32 {
    let argument = door_windows_abi::widen(argument);
    {
        let timer = unsafe { &*(argument as *const Timer) };
        let mut next = tick() + u64::from(timer.delay);
        while timer.live.load(Ordering::Acquire) {
            let now = tick();
            if now < next {
                unsafe { DoorSleep(((next - now).min(5)) as u32) };
                continue;
            }
            timer.running.store(true, Ordering::Release);
            if timer.live.load(Ordering::Acquire) {
                if timer.flags & (TIME_CALLBACK_EVENT_SET | TIME_CALLBACK_EVENT_PULSE) != 0 {
                    unsafe { DoorSetEvent(timer.callback) };
                } else {
                    let call: TimeProc =
                        unsafe { core::mem::transmute::<usize, TimeProc>(timer.callback as usize) };
                    call(timer.id, 0, timer.user as usize, 0, 0);
                }
            }
            timer.running.store(false, Ordering::Release);
            if timer.flags & TIME_PERIODIC == 0 {
                timer.live.store(false, Ordering::Release);
                break;
            }
            next += u64::from(timer.delay.max(1));
        }
        0
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn timeSetEvent(
    delay: u32,
    _resolution: u32,
    callback: usize,
    user: usize,
    flags: u32,
) -> u32 {
    let callback = door_windows_abi::widen(callback);
    let user = door_windows_abi::widen(user);
    {
        if callback == 0 || !(1..=1_000_000).contains(&delay) {
            return 0;
        }
        let id = NEXT_TIMER.fetch_add(1, Ordering::Relaxed);
        let timer: &'static Timer = Box::leak(Box::new(Timer {
            id,
            delay,
            callback,
            user,
            flags,
            live: AtomicBool::new(true),
            running: AtomicBool::new(false),
        }));
        let thread = unsafe {
            DoorCreateThreadEx(
                timer_thread as *const () as u64,
                timer as *const Timer as u64,
                0,
            )
        };
        if thread == 0 {
            return 0;
        }
        unsafe { DoorCloseHandle(thread) };
        TIMERS.with(|t| t.push(timer));
        id
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn timeKillEvent(id: u32) -> u32 {
    let Some(timer) = TIMERS.with(|t| t.iter().position(|x| x.id == id).map(|i| t.remove(i)))
    else {
        return mm::INVALPARAM;
    };
    timer.live.store(false, Ordering::Release);
    if timer.flags & TIME_KILL_SYNCHRONOUS != 0 {
        while timer.running.load(Ordering::Acquire) {
            unsafe { DoorSleep(1) };
        }
    }
    TIMERR_NOERROR
}
