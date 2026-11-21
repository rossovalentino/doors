//! MIDI output: the synthesizer device (midiOut*) and MIDI streams
//! (midiStream*). A process opens it once at a time (Windows answers
//! MMSYSERR_ALLOCATED to a second open); the worker renders the synthesizer
//! into a door.audio stream in 20 ms sections, two queued, so short and
//! long messages sound in the next section and stream events at their
//! sample. MOM_DONE and MOM_POSITIONCB are reported when door.audio has
//! played the frame where their buffer ended or their event fell.
use super::*;
use crate::synth::{RATE, Synth};

const MOM_OPEN: u32 = 0x3C7;
const MOM_CLOSE: u32 = 0x3C8;
const MOM_DONE: u32 = 0x3C9;
const MOM_POSITIONCB: u32 = 0x3CA;
const MHDR_DONE: u32 = 0x1;
const MHDR_PREPARED: u32 = 0x2;
const MHDR_INQUEUE: u32 = 0x4;
const MHDR_ISSTRM: u32 = 0x8;
const MIDIERR_UNPREPARED: u32 = 64;
const MIDIERR_STILLPLAYING: u32 = 65;
const MIDI_MAPPER: u32 = u32::MAX;
/// MIDIHDR (mmsystem.h packs it): lpData, dwBufferLength, dwBytesRecorded,
/// dwUser, dwFlags, lpNext, reserved, dwOffset, dwReserved[8] (DWORD_PTRs):
/// 112 bytes on x64 (dwOffset at 44), 64 on x86 (at 28). Up to and
/// including dwOffset is the least a caller passes.
const MIDIHDR: [door_windows_abi::layout::Field; 8] = {
    use door_windows_abi::layout::Field::*;
    [Pointer, U32, U32, Pointer, U32, Pointer, Pointer, U32]
};
const HEADER_MIN: u32 = door_windows_abi::layout::size_pack(&MIDIHDR, 1) as u32;
const OFFSET: u64 = door_windows_abi::layout::offset_pack(&MIDIHDR, 7, 1) as u64;
const FLAGS: u64 = door_windows_abi::layout::offset_pack(&MIDIHDR, 4, 1) as u64;
const LENGTH: u64 = door_windows_abi::layout::offset_pack(&MIDIHDR, 1, 1) as u64;
const RECORDED: u64 = door_windows_abi::layout::offset_pack(&MIDIHDR, 2, 1) as u64;
const MIDI_IO_STATUS: u32 = 0x20;
const MEVT_F_LONG: u32 = 0x8000_0000;
const MEVT_F_CALLBACK: u32 = 0x4000_0000;
const MEVT_SHORTMSG: u8 = 0x00;
const MEVT_TEMPO: u8 = 0x01;
const MEVT_LONGMSG: u8 = 0x80;
const MIDIPROP_SET: u32 = 0x8000_0000;
const MIDIPROP_GET: u32 = 0x4000_0000;
const MIDIPROP_TIMEDIV: u32 = 1;
const MIDIPROP_TEMPO: u32 = 2;
const TIME_MS: u32 = 1;
const TIME_TICKS: u32 = 0x20;
/// 20 ms of the synthesizer's output per section.
const SECTION_FRAMES: usize = RATE as usize / 50;
/// Sections rendered ahead of what door.audio has played (20 ms each):
/// 120 ms, as Windows' software synthesizer runs. With two (40 ms) the
/// worker fell behind whenever it was not scheduled for about 35 ms, and the
/// stream ran dry mid-note (audible as clicks; P15 MIDI-A, 14 November 2026).
/// Positions and callbacks follow what was played, not what was rendered.
const MIDI_QUEUED_SECTIONS: usize = 6;

/// A stream event, copied from the application's MIDIEVENT records when
/// its buffer is queued (the buffer is the driver's until MHDR_DONE).
#[derive(Clone, Debug)]
enum Kind {
    Short(u32),
    Tempo(u32),
    Long(Vec<u8>),
    Nop,
}
#[derive(Clone, Debug)]
struct Event {
    delta: u32,
    kind: Kind,
    /// Byte offset in the buffer (dwOffset for MOM_POSITIONCB).
    offset: u32,
    callback: bool,
}
/// Parses a buffer of MIDIEVENT records (dwDeltaTime, dwStreamID, dwEvent
/// and, for long events, their parameters padded to a DWORD).
fn events(data: &[u8]) -> Vec<Event> {
    let mut out = Vec::new();
    let mut at = 0usize;
    while at + 12 <= data.len() {
        let word = |i: usize| u32::from_le_bytes(data[i..i + 4].try_into().unwrap());
        let (delta, event) = (word(at), word(at + 8));
        let callback = event & MEVT_F_CALLBACK != 0;
        let kind_byte = ((event >> 24) as u8) & 0xBF;
        let offset = at as u32;
        at += 12;
        let kind = if event & MEVT_F_LONG != 0 {
            let length = (event & 0x00FF_FFFF) as usize;
            let Some(params) = data.get(at..at + length) else {
                break;
            };
            at += length.div_ceil(4) * 4;
            match kind_byte {
                MEVT_LONGMSG => Kind::Long(params.to_vec()),
                _ => Kind::Nop,
            }
        } else {
            match kind_byte {
                MEVT_SHORTMSG => Kind::Short(event & 0x00FF_FFFF),
                MEVT_TEMPO => Kind::Tempo(event & 0x00FF_FFFF),
                _ => Kind::Nop,
            }
        };
        out.push(Event {
            delta,
            kind,
            offset,
            callback,
        });
    }
    out
}
struct Queued {
    header: u64,
    events: Vec<Event>,
    next: usize,
}
/// What a stream handle adds to the device.
struct Sequence {
    division: u32,
    tempo: u32,
    running: bool,
    buffers: VecDeque<Queued>,
    /// Units left before the next event, where a frame is
    /// `frame_units()` and a tick `tick_units()`; None when no event is
    /// loaded.
    wait: Option<u128>,
    /// Units the loaded event's delta took (for partial ticks).
    loaded: u128,
    /// Ticks of the events processed.
    ticks: u64,
    /// Frames rendered while running.
    frames: u64,
    /// (device frame at a section's end, stream frames, ticks) for the
    /// position door.audio has played.
    marks: VecDeque<(u64, u64, u64)>,
}
impl Sequence {
    fn new() -> Self {
        Self {
            division: 24,
            tempo: 500_000,
            running: false,
            buffers: VecDeque::new(),
            wait: None,
            loaded: 0,
            ticks: 0,
            frames: 0,
            marks: VecDeque::new(),
        }
    }
    fn smpte(&self) -> bool {
        self.division & 0x8000 != 0
    }
    /// SMPTE divisions: frames per second times ticks per frame.
    fn ticks_per_second(&self) -> u128 {
        let fps = u128::from(((self.division >> 8) as u8 as i8).unsigned_abs());
        (fps * u128::from(self.division & 0xFF)).max(1)
    }
    fn frame_units(&self) -> u128 {
        if self.smpte() {
            self.ticks_per_second() * 1_000_000
        } else {
            u128::from(self.division.max(1)) * 1_000_000
        }
    }
    fn tick_units(&self) -> u128 {
        if self.smpte() {
            u128::from(RATE) * 1_000_000
        } else {
            u128::from(self.tempo) * u128::from(RATE)
        }
    }
    /// Ticks played including the part of the current wait.
    fn ticks_now(&self) -> u64 {
        let partial = match self.wait {
            Some(w) => (self.loaded.saturating_sub(w)) / self.tick_units().max(1),
            None => 0,
        };
        self.ticks + partial as u64
    }
}
/// A report due when door.audio has played to a frame.
enum Due {
    Done(u64),
    Position(u64, u32),
}
struct Out {
    handle: u64,
    callback: (u32, u64, u64),
    synth: Synth,
    stream: Option<u32>,
    /// Frames handed to door.audio, and the end of each queued section.
    rendered: u64,
    sections: VecDeque<u64>,
    /// A rendered section door.audio refused (it goes first next time).
    pending: Option<Vec<u8>>,
    volume: u32,
    sequence: Option<Sequence>,
    due: VecDeque<(u64, Due)>,
    /// Notes to report outside the lock (stops and resets).
    immediate: Vec<Note>,
}
static MIDI: Lock<Option<Out>> = Lock::new(None);

fn with_out<R>(handle: u64, f: impl FnOnce(&mut Out) -> Result<R, u32>) -> Result<R, u32> {
    MIDI.with(|m| match m.as_mut() {
        Some(o) if o.handle == handle && handle != 0 => f(o),
        _ => Err(mm::INVALHANDLE),
    })
}
fn header_flags(header: u64) -> Result<u32, u32> {
    read_u32(header + FLAGS)
}
fn set_flags(header: u64, set: u32, clear: u32) {
    if let Ok(flags) = read_u32(header + FLAGS) {
        let _ = write_u32(header + FLAGS, (flags | set) & !clear);
    }
}
fn note(o: &Out, message: u32, header: u64) -> Note {
    Note {
        device: o.handle,
        callback: o.callback,
        message,
        header,
    }
}
/// Every channel quiet: all notes off and the sustain pedal up.
fn notes_off(synth: &mut Synth) {
    for channel in 0..16u8 {
        synth.short(0xB0 | channel, 64, 0);
        synth.short(0xB0 | channel, 123, 0);
    }
}
/// The stream's queued buffers back to the application, done.
fn return_buffers(o: &mut Out) {
    let Some(s) = o.sequence.as_mut() else {
        return;
    };
    let headers: Vec<u64> = s.buffers.drain(..).map(|b| b.header).collect();
    s.wait = None;
    // Reports still waiting for door.audio go now too.
    let mut reported: Vec<u64> = o
        .due
        .drain(..)
        .filter_map(|(_, d)| match d {
            Due::Done(h) => Some(h),
            Due::Position(..) => None,
        })
        .collect();
    reported.extend(headers);
    for header in reported {
        set_flags(header, MHDR_DONE, MHDR_INQUEUE);
        let n = note(o, MOM_DONE, header);
        o.immediate.push(n);
    }
}

// ---- rendering ---------------------------------------------------------------
fn apply(synth: &mut Synth, kind: &Kind, sequence: &mut Sequence) {
    match kind {
        Kind::Short(word) => synth.message(*word),
        Kind::Tempo(t) => sequence.tempo = (*t).max(1),
        Kind::Long(data) => {
            if data.first() == Some(&0xF0) {
                synth.sysex(data);
            }
        }
        Kind::Nop => {}
    }
}
/// Loads the next event's wait (from the front buffer).
fn load(s: &mut Sequence) {
    if s.wait.is_some() {
        return;
    }
    if let Some(b) = s.buffers.front()
        && let Some(e) = b.events.get(b.next)
    {
        let units = u128::from(e.delta) * s.tick_units();
        s.wait = Some(units);
        s.loaded = units;
    }
}
/// One section of output: the synthesizer between stream events.
fn render(o: &mut Out) -> Vec<u8> {
    let mut pcm = vec![0i16; SECTION_FRAMES * 2];
    let mut done = 0usize;
    while done < SECTION_FRAMES {
        let mut run = SECTION_FRAMES - done;
        if let Some(s) = o.sequence.as_mut()
            && s.running
        {
            // Events due now.
            loop {
                load(s);
                let Some(0) = s.wait else { break };
                let frame = o.rendered + done as u64;
                let Some(b) = s.buffers.front_mut() else {
                    break;
                };
                let e = b.events[b.next].clone();
                b.next += 1;
                let header = b.header;
                let last = b.next >= b.events.len();
                if last {
                    s.buffers.pop_front();
                }
                s.ticks += u64::from(e.delta);
                s.wait = None;
                apply(&mut o.synth, &e.kind, s);
                if e.callback {
                    o.due.push_back((frame, Due::Position(header, e.offset)));
                }
                if last {
                    o.due.push_back((frame, Due::Done(header)));
                }
            }
            if let Some(w) = s.wait {
                let frames = w.div_ceil(s.frame_units());
                run = run.min(usize::try_from(frames).unwrap_or(usize::MAX).max(1));
            }
        }
        o.synth.render(&mut pcm[2 * done..2 * (done + run)]);
        if let Some(s) = o.sequence.as_mut()
            && s.running
        {
            s.frames += run as u64;
            let elapsed = run as u128 * s.frame_units();
            if let Some(w) = s.wait.as_mut() {
                *w = w.saturating_sub(elapsed);
            }
        }
        done += run;
    }
    // midiOutSetVolume and the application's volume.
    let app = APP_VOLUME.load(Ordering::Relaxed);
    let gain =
        |shift: u32| u64::from((o.volume >> shift) & 0xFFFF) * u64::from((app >> shift) & 0xFFFF);
    let (left, right) = (gain(0), gain(16));
    let full = 0xFFFFu64 * 0xFFFF;
    let mut bytes = vec![0u8; SECTION_FRAMES * 4];
    for (i, s) in pcm.iter().enumerate() {
        let g = if i % 2 == 0 { left } else { right };
        let v = if g == full {
            *s
        } else {
            (i64::from(*s) * g as i64 / full as i64) as i16
        };
        bytes[2 * i..2 * i + 2].copy_from_slice(&v.to_le_bytes());
    }
    bytes
}
/// Whether the device has sound to make: voices, or a running stream
/// with events.
fn busy(o: &Out) -> bool {
    o.synth.active() > 0
        || o.sequence
            .as_ref()
            .is_some_and(|s| s.running && !s.buffers.is_empty())
}
/// The worker's pass: retire what door.audio played, report it, render.
pub(super) fn pass() -> Vec<Note> {
    let stream = MIDI.with(|m| m.as_ref().and_then(|o| o.stream));
    let played = stream.and_then(super::played).map(|(f, _)| f);
    MIDI.with(|m| {
        let Some(o) = m.as_mut() else {
            return Vec::new();
        };
        let mut notes = core::mem::take(&mut o.immediate);
        if let Some(played) = played {
            while o.sections.front().is_some_and(|end| *end <= played) {
                o.sections.pop_front();
            }
            while o.due.front().is_some_and(|(frame, _)| *frame <= played) {
                let (_, d) = o.due.pop_front().unwrap();
                match d {
                    Due::Done(header) => {
                        set_flags(header, MHDR_DONE, MHDR_INQUEUE);
                        notes.push(note(o, MOM_DONE, header));
                    }
                    Due::Position(header, offset) => {
                        let _ = write_u32(header + OFFSET, offset);
                        notes.push(note(o, MOM_POSITIONCB, header));
                    }
                }
            }
            if let Some(s) = o.sequence.as_mut() {
                while s.marks.len() > 1 && s.marks[1].0 <= played {
                    s.marks.pop_front();
                }
            }
        }
        while o.sections.len() < MIDI_QUEUED_SECTIONS && (o.pending.is_some() || busy(o)) {
            if o.stream.is_none() {
                let format = Format {
                    rate: RATE,
                    channels: 2,
                    bits: 16,
                };
                match audio(|a| a.open(format)) {
                    Some(Ok(id)) => o.stream = Some(id),
                    _ => break,
                }
            }
            let pcm = match o.pending.take() {
                Some(pcm) => pcm,
                None => {
                    let pcm = render(o);
                    o.rendered += SECTION_FRAMES as u64;
                    let end = o.rendered;
                    if let Some(s) = o.sequence.as_mut() {
                        let ticks = s.ticks_now();
                        s.marks.push_back((end, s.frames, ticks));
                    }
                    pcm
                }
            };
            let stream = o.stream.unwrap_or(0);
            let queued = super::section(pcm.len(), |address| unsafe {
                core::ptr::copy_nonoverlapping(pcm.as_ptr(), address as *mut u8, pcm.len());
            })
            .is_some_and(|handle| {
                let ok = audio(|a| a.queue(stream, handle, pcm.len()).is_ok()).unwrap_or(false);
                let _ = door_sys::close(handle);
                ok
            });
            if queued {
                o.sections.push_back(o.rendered);
            } else {
                o.pending = Some(pcm);
                break;
            }
        }
        notes
    })
}

// ---- midiOut -------------------------------------------------------------------
#[unsafe(no_mangle)]
pub extern "system" fn midiOutGetNumDevs() -> u32 {
    u32::from(device_present())
}
/// MIDIOUTCAPS: wMid, wPid, vDriverVersion, szPname[32], wTechnology,
/// wVoices, wNotes, wChannelMask, dwSupport (measured on Windows 11:
/// the synthesizer is MOD_SWSYNTH with 32 voices and volume; the mapper
/// MOD_MAPPER with volume and streams).
fn midi_caps(device: u32, wide: bool) -> Result<Vec<u8>, u32> {
    if !device_present() || (device != 0 && device != MIDI_MAPPER) {
        return Err(mm::BADDEVICEID);
    }
    let (pid, version, name, technology, voices, support): (u16, u32, &str, u16, u16, u32) =
        if device == MIDI_MAPPER {
            (1, 0x0500, "Macrohard MIDI Mapper", 5, 0, 9)
        } else {
            (27, 0x0100, "Macrohard GS Wavetable Synth", 7, 32, 1)
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
    b.extend_from_slice(&technology.to_le_bytes());
    b.extend_from_slice(&voices.to_le_bytes());
    b.extend_from_slice(&voices.to_le_bytes());
    b.extend_from_slice(&0xFFFFu16.to_le_bytes());
    b.extend_from_slice(&support.to_le_bytes());
    Ok(b)
}
fn get_midi_caps(device: u64, out: u64, size: u32, wide: bool) -> u32 {
    match midi_caps(device as u32, wide) {
        Ok(b) => {
            let n = (size as usize).min(b.len());
            write(out, &b[..n]).err().unwrap_or(mm::NOERROR)
        }
        Err(e) => e,
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn midiOutGetDevCapsW(device: usize, out: usize, size: u32) -> u32 {
    let device = door_windows_abi::widen(device);
    let out = door_windows_abi::widen(out);
    get_midi_caps(device, out, size, true)
}
#[unsafe(no_mangle)]
pub extern "system" fn midiOutGetDevCapsA(device: usize, out: usize, size: u32) -> u32 {
    let device = door_windows_abi::widen(device);
    let out = door_windows_abi::widen(out);
    get_midi_caps(device, out, size, false)
}
fn open(
    out: u64,
    device: u32,
    callback: u64,
    instance: u64,
    flags: u32,
    sequence: Option<Sequence>,
) -> u32 {
    let result = (|| {
        if device != 0 && device != MIDI_MAPPER {
            return Err(mm::BADDEVICEID);
        }
        if !device_present() {
            return Err(mm::NODRIVER);
        }
        let kind = flags & CALLBACK_TYPEMASK;
        if !matches!(
            kind,
            CALLBACK_NULL | CALLBACK_WINDOW | CALLBACK_THREAD | CALLBACK_FUNCTION | CALLBACK_EVENT
        ) || flags & !(CALLBACK_TYPEMASK | MIDI_IO_STATUS) != 0
        {
            return Err(mm::INVALFLAG);
        }
        if out == 0 {
            return Err(mm::INVALPARAM);
        }
        let handle = NEXT_HANDLE.fetch_add(0x10, Ordering::Relaxed);
        let opened = MIDI.with(|m| {
            if m.is_some() {
                return Err(mm::ALLOCATED);
            }
            *m = Some(Out {
                handle,
                callback: (kind, callback, instance),
                synth: Synth::new(),
                stream: None,
                rendered: 0,
                sections: VecDeque::new(),
                pending: None,
                volume: 0xFFFF_FFFF,
                sequence,
                due: VecDeque::new(),
                immediate: Vec::new(),
            });
            Ok(())
        });
        opened?;
        if super::write_pointer(out, handle).is_err() {
            MIDI.with(|m| *m = None);
            return Err(mm::INVALPARAM);
        }
        start_worker();
        Ok(handle)
    })();
    match result {
        Ok(handle) => {
            deliver(&Note {
                device: handle,
                callback: ((flags & CALLBACK_TYPEMASK), callback, instance),
                message: MOM_OPEN,
                header: 0,
            });
            mm::NOERROR
        }
        Err(e) => e,
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn midiOutOpen(
    out: usize,
    device: u32,
    callback: usize,
    instance: usize,
    flags: u32,
) -> u32 {
    let out = door_windows_abi::widen(out);
    let callback = door_windows_abi::widen(callback);
    let instance = door_windows_abi::widen(instance);
    open(out, device, callback, instance, flags, None)
}
fn close(handle: u64) -> u32 {
    let closed = MIDI.with(|m| match m.as_ref() {
        Some(o) if o.handle == handle && handle != 0 => {
            if o.sequence.as_ref().is_some_and(|s| !s.buffers.is_empty()) {
                return Err(MIDIERR_STILLPLAYING);
            }
            Ok(m.take().unwrap())
        }
        _ => Err(mm::INVALHANDLE),
    });
    match closed {
        Ok(o) => {
            if let Some(stream) = o.stream {
                audio(|a| a.close(stream));
            }
            for n in &o.immediate {
                deliver(n);
            }
            deliver(&note(&o, MOM_CLOSE, 0));
            mm::NOERROR
        }
        Err(e) => e,
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn midiOutClose(handle: usize) -> u32 {
    let handle = door_windows_abi::widen(handle);
    close(handle)
}
#[unsafe(no_mangle)]
pub extern "system" fn midiOutShortMsg(handle: usize, message: u32) -> u32 {
    let handle = door_windows_abi::widen(handle);
    {
        with_out(handle, |o| {
            o.synth.message(message);
            Ok(())
        })
        .err()
        .unwrap_or(mm::NOERROR)
    }
}
fn check_header(header: u64, size: u32) -> Result<(), u32> {
    if header == 0 || size < HEADER_MIN {
        return Err(mm::INVALPARAM);
    }
    Ok(())
}
#[unsafe(no_mangle)]
pub extern "system" fn midiOutPrepareHeader(handle: usize, header: usize, size: u32) -> u32 {
    let handle = door_windows_abi::widen(handle);
    let header = door_windows_abi::widen(header);
    {
        let result = with_out(handle, |_| Ok(())).and_then(|()| {
            check_header(header, size)?;
            let flags = header_flags(header)?;
            write_u32(header + FLAGS, flags | MHDR_PREPARED)
        });
        result.err().unwrap_or(mm::NOERROR)
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn midiOutUnprepareHeader(handle: usize, header: usize, size: u32) -> u32 {
    let handle = door_windows_abi::widen(handle);
    let header = door_windows_abi::widen(header);
    {
        let result = with_out(handle, |_| Ok(())).and_then(|()| {
            check_header(header, size)?;
            let flags = header_flags(header)?;
            if flags & MHDR_INQUEUE != 0 {
                return Err(MIDIERR_STILLPLAYING);
            }
            write_u32(header + FLAGS, flags & !MHDR_PREPARED)
        });
        result.err().unwrap_or(mm::NOERROR)
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn midiOutLongMsg(handle: usize, header: usize, size: u32) -> u32 {
    let handle = door_windows_abi::widen(handle);
    let header = door_windows_abi::widen(header);
    {
        let result = (|| {
            let callback = with_out(handle, |o| Ok(o.callback))?;
            check_header(header, size)?;
            let flags = header_flags(header)?;
            if flags & MHDR_PREPARED == 0 {
                return Err(MIDIERR_UNPREPARED);
            }
            let data = super::read_pointer(header)?;
            let length = read_u32(header + LENGTH)?.min(64 * 1024);
            let mut bytes = vec![0u8; length as usize];
            read(data, &mut bytes)?;
            with_out(handle, |o| {
                // Each F0..F7 message in the buffer.
                let mut at = 0;
                while let Some(start) = bytes[at..].iter().position(|b| *b == 0xF0) {
                    let begin = at + start;
                    let end = bytes[begin..]
                        .iter()
                        .position(|b| *b == 0xF7)
                        .map_or(bytes.len(), |e| begin + e + 1);
                    o.synth.sysex(&bytes[begin..end]);
                    at = end;
                }
                Ok(())
            })?;
            // Sent: done at once, as Windows' synthesizer reports it.
            write_u32(header + FLAGS, (flags | MHDR_DONE) & !MHDR_INQUEUE)?;
            deliver(&Note {
                device: handle,
                callback,
                message: MOM_DONE,
                header,
            });
            Ok(())
        })();
        result.err().unwrap_or(mm::NOERROR)
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn midiOutReset(handle: usize) -> u32 {
    let handle = door_windows_abi::widen(handle);
    {
        let result = with_out(handle, |o| {
            notes_off(&mut o.synth);
            return_buffers(o);
            Ok(core::mem::take(&mut o.immediate))
        });
        match result {
            Ok(notes) => {
                notes.iter().for_each(deliver);
                mm::NOERROR
            }
            Err(e) => e,
        }
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn midiOutGetVolume(handle: usize, out: usize) -> u32 {
    let handle = door_windows_abi::widen(handle);
    let out = door_windows_abi::widen(out);
    {
        let volume = if handle == 0 || handle == u64::from(MIDI_MAPPER) || handle == u64::MAX {
            if device_present() {
                Ok(APP_VOLUME.load(Ordering::Relaxed))
            } else {
                Err(mm::NODRIVER)
            }
        } else {
            with_out(handle, |o| Ok(o.volume))
        };
        volume
            .and_then(|v| write_u32(out, v))
            .err()
            .unwrap_or(mm::NOERROR)
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn midiOutSetVolume(handle: usize, volume: u32) -> u32 {
    let handle = door_windows_abi::widen(handle);
    {
        if handle == 0 || handle == u64::from(MIDI_MAPPER) || handle == u64::MAX {
            return super::waveOutSetVolume(door_windows_abi::narrow(handle), volume);
        }
        with_out(handle, |o| {
            o.volume = volume;
            Ok(())
        })
        .err()
        .unwrap_or(mm::NOERROR)
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn midiOutGetID(handle: usize, out: usize) -> u32 {
    let handle = door_windows_abi::widen(handle);
    let out = door_windows_abi::widen(out);
    {
        with_out(handle, |_| Ok(()))
            .and_then(|()| write_u32(out, 0))
            .err()
            .unwrap_or(mm::NOERROR)
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn midiOutGetErrorTextW(code: u32, out: usize, length: u32) -> u32 {
    let out = door_windows_abi::widen(out);
    super::error_text(code, out, length, true)
}
#[unsafe(no_mangle)]
pub extern "system" fn midiOutGetErrorTextA(code: u32, out: usize, length: u32) -> u32 {
    let out = door_windows_abi::widen(out);
    super::error_text(code, out, length, false)
}
#[unsafe(no_mangle)]
pub extern "system" fn midiOutMessage(_handle: usize, _message: u32, _a: usize, _b: usize) -> u32 {
    mm::NOTSUPPORTED
}
/// The synthesizer does not cache patches (no MIDICAPS_CACHE).
#[unsafe(no_mangle)]
pub extern "system" fn midiOutCachePatches(
    _handle: usize,
    _bank: u32,
    _patches: usize,
    _flags: u32,
) -> u32 {
    mm::NOTSUPPORTED
}
#[unsafe(no_mangle)]
pub extern "system" fn midiOutCacheDrumPatches(
    _handle: usize,
    _patch: u32,
    _keys: usize,
    _flags: u32,
) -> u32 {
    mm::NOTSUPPORTED
}

// ---- midiStream ----------------------------------------------------------------
#[unsafe(no_mangle)]
pub extern "system" fn midiStreamOpen(
    out: usize,
    device: usize,
    count: u32,
    callback: usize,
    instance: usize,
    flags: u32,
) -> u32 {
    let out = door_windows_abi::widen(out);
    let device = door_windows_abi::widen(device);
    let callback = door_windows_abi::widen(callback);
    let instance = door_windows_abi::widen(instance);
    {
        if count != 1 || device == 0 {
            return mm::INVALPARAM;
        }
        let Ok(id) = read_u32(device) else {
            return mm::INVALPARAM;
        };
        open(out, id, callback, instance, flags, Some(Sequence::new()))
    }
}
fn with_sequence<R>(handle: u64, f: impl FnOnce(&mut Out) -> Result<R, u32>) -> Result<R, u32> {
    with_out(handle, |o| {
        if o.sequence.is_none() {
            return Err(mm::INVALHANDLE);
        }
        f(o)
    })
}
#[unsafe(no_mangle)]
pub extern "system" fn midiStreamClose(handle: usize) -> u32 {
    let handle = door_windows_abi::widen(handle);
    {
        if with_sequence(handle, |_| Ok(())).is_err() {
            return mm::INVALHANDLE;
        }
        close(handle)
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn midiStreamOut(handle: usize, header: usize, size: u32) -> u32 {
    let handle = door_windows_abi::widen(handle);
    let header = door_windows_abi::widen(header);
    {
        let result = (|| {
            with_sequence(handle, |_| Ok(()))?;
            check_header(header, size)?;
            let flags = header_flags(header)?;
            if flags & MHDR_PREPARED == 0 {
                return Err(MIDIERR_UNPREPARED);
            }
            if flags & MHDR_INQUEUE != 0 {
                return Err(MIDIERR_STILLPLAYING);
            }
            let data = super::read_pointer(header)?;
            let length = read_u32(header + LENGTH)?;
            let recorded = read_u32(header + RECORDED)?;
            if recorded > length || recorded % 4 != 0 {
                return Err(mm::INVALPARAM);
            }
            let mut bytes = vec![0u8; recorded as usize];
            read(data, &mut bytes)?;
            let parsed = events(&bytes);
            write_u32(
                header + FLAGS,
                (flags | MHDR_INQUEUE | MHDR_ISSTRM) & !MHDR_DONE,
            )?;
            with_sequence(handle, |o| {
                let empty = parsed.is_empty();
                let s = o.sequence.as_mut().unwrap();
                if empty {
                    // Nothing to play: done when everything before it is.
                    o.due.push_back((o.rendered, Due::Done(header)));
                } else {
                    s.buffers.push_back(Queued {
                        header,
                        events: parsed,
                        next: 0,
                    });
                }
                Ok(())
            })
        })();
        result.err().unwrap_or(mm::NOERROR)
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn midiStreamRestart(handle: usize) -> u32 {
    let handle = door_windows_abi::widen(handle);
    {
        with_sequence(handle, |o| {
            o.sequence.as_mut().unwrap().running = true;
            Ok(())
        })
        .err()
        .unwrap_or(mm::NOERROR)
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn midiStreamPause(handle: usize) -> u32 {
    let handle = door_windows_abi::widen(handle);
    {
        with_sequence(handle, |o| {
            let s = o.sequence.as_mut().unwrap();
            if s.running {
                s.running = false;
                notes_off(&mut o.synth);
            }
            Ok(())
        })
        .err()
        .unwrap_or(mm::NOERROR)
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn midiStreamStop(handle: usize) -> u32 {
    let handle = door_windows_abi::widen(handle);
    {
        let result = with_sequence(handle, |o| {
            notes_off(&mut o.synth);
            return_buffers(o);
            let s = o.sequence.as_mut().unwrap();
            s.running = false;
            s.ticks = 0;
            s.frames = 0;
            s.marks.clear();
            Ok(core::mem::take(&mut o.immediate))
        });
        match result {
            Ok(notes) => {
                notes.iter().for_each(deliver);
                mm::NOERROR
            }
            Err(e) => e,
        }
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn midiStreamPosition(handle: usize, time: usize, size: u32) -> u32 {
    let handle = door_windows_abi::widen(handle);
    let time = door_windows_abi::widen(time);
    {
        let result = (|| {
            if time == 0 || size < 8 {
                return Err(mm::INVALPARAM);
            }
            let kind = read_u32(time)?;
            let stream = with_sequence(handle, |o| Ok(o.stream))?;
            let played = stream.and_then(super::played).map_or(0, |(f, _)| f);
            let (frames, ticks) = with_sequence(handle, |o| {
                let s = o.sequence.as_ref().unwrap();
                // Interpolate between the marks around what door.audio played.
                let mut at = (0u64, 0u64, 0u64);
                let mut next = None;
                for m in &s.marks {
                    if m.0 <= played {
                        at = *m;
                    } else {
                        next = Some(*m);
                        break;
                    }
                }
                Ok(match next {
                    Some(n) if n.0 > at.0 && played > at.0 => {
                        let f = (played - at.0) as u128;
                        let span = (n.0 - at.0) as u128;
                        (
                            at.1 + ((n.1 - at.1) as u128 * f / span) as u64,
                            at.2 + ((n.2 - at.2) as u128 * f / span) as u64,
                        )
                    }
                    _ => (at.1, at.2),
                })
            })?;
            // TIME_TICKS as asked; everything else in milliseconds (Windows
            // answers TIME_MS for the types a stream does not keep).
            let (kind, value) = if kind == TIME_TICKS {
                (TIME_TICKS, ticks as u32)
            } else {
                (TIME_MS, (frames * 1000 / u64::from(RATE)) as u32)
            };
            let mut b = [0u8; 8];
            b[..4].copy_from_slice(&kind.to_le_bytes());
            b[4..].copy_from_slice(&value.to_le_bytes());
            write(time, &b)
        })();
        result.err().unwrap_or(mm::NOERROR)
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn midiStreamProperty(handle: usize, data: usize, property: u32) -> u32 {
    let handle = door_windows_abi::widen(handle);
    let data = door_windows_abi::widen(data);
    {
        let result = (|| {
            let which = property & !(MIDIPROP_GET | MIDIPROP_SET);
            let get = property & MIDIPROP_GET != 0;
            if get == (property & MIDIPROP_SET != 0)
                || !matches!(which, MIDIPROP_TIMEDIV | MIDIPROP_TEMPO)
            {
                return Err(mm::INVALFLAG);
            }
            // MIDIPROPTIMEDIV / MIDIPROPTEMPO: cbStruct, then the value.
            if read_u32(data)? < 8 {
                return Err(mm::INVALPARAM);
            }
            if get {
                let value = with_sequence(handle, |o| {
                    let s = o.sequence.as_ref().unwrap();
                    Ok(if which == MIDIPROP_TIMEDIV {
                        s.division
                    } else {
                        s.tempo
                    })
                })?;
                write_u32(data + 4, value)
            } else {
                let value = read_u32(data + 4)?;
                with_sequence(handle, |o| {
                    let s = o.sequence.as_mut().unwrap();
                    if which == MIDIPROP_TIMEDIV {
                        // The division changes only while stopped.
                        if s.running || value == 0 {
                            return Err(mm::INVALPARAM);
                        }
                        s.division = value;
                    } else {
                        s.tempo = value.max(1);
                    }
                    Ok(())
                })
            }
        })();
        result.err().unwrap_or(mm::NOERROR)
    }
}
/// MIDI thru connections need a MIDI input; Doors has none.
#[unsafe(no_mangle)]
pub extern "system" fn midiConnect(_input: usize, _output: usize, _reserved: usize) -> u32 {
    mm::NOTSUPPORTED
}
#[unsafe(no_mangle)]
pub extern "system" fn midiDisconnect(_input: usize, _output: usize, _reserved: usize) -> u32 {
    mm::NOTSUPPORTED
}

// ---- MCI's sequencer ------------------------------------------------------------
/// Opens the device for the MCI sequencer (as an application would open a
/// stream); fails with ALLOCATED while the application holds it.
pub(super) fn sequencer_open() -> Result<u64, u32> {
    let handle = NEXT_HANDLE.fetch_add(0x10, Ordering::Relaxed);
    MIDI.with(|m| {
        if m.is_some() {
            return Err(mm::ALLOCATED);
        }
        let mut sequence = Sequence::new();
        sequence.division = 0;
        *m = Some(Out {
            handle,
            callback: (CALLBACK_NULL, 0, 0),
            synth: Synth::new(),
            stream: None,
            rendered: 0,
            sections: VecDeque::new(),
            pending: None,
            volume: 0xFFFF_FFFF,
            sequence: Some(sequence),
            due: VecDeque::new(),
            immediate: Vec::new(),
        });
        Ok(())
    })?;
    start_worker();
    Ok(handle)
}
/// Queues a song's events (from `first` on, as (delta, kind)) and starts
/// them; the sequencer's buffers have no application header.
pub(super) fn sequencer_play(
    handle: u64,
    division: u16,
    song: &[(u32, crate::smf::Event)],
) -> Result<(), u32> {
    with_sequence(handle, |o| {
        notes_off(&mut o.synth);
        let s = o.sequence.as_mut().unwrap();
        s.buffers.clear();
        s.wait = None;
        s.division = u32::from(division);
        s.tempo = 500_000;
        s.ticks = 0;
        s.frames = 0;
        s.marks.clear();
        let events = song
            .iter()
            .map(|(delta, e)| Event {
                delta: *delta,
                kind: match e {
                    crate::smf::Event::Short(w) => Kind::Short(*w),
                    crate::smf::Event::Sysex(d) => Kind::Long(d.clone()),
                    crate::smf::Event::Tempo(t) => Kind::Tempo(*t),
                },
                offset: 0,
                callback: false,
            })
            .collect::<Vec<_>>();
        if !events.is_empty() {
            s.buffers.push_back(Queued {
                header: 0,
                events,
                next: 0,
            });
        }
        s.running = true;
        // The reported end is the sequencer's: drop the Done note for
        // header 0 (no application header).
        Ok(())
    })
}
/// The sequencer's state: (frames played, finished).
pub(super) fn sequencer_state(handle: u64) -> Option<(u64, bool)> {
    let stream = with_sequence(handle, |o| Ok(o.stream)).ok()?;
    let played = stream.and_then(super::played);
    with_sequence(handle, |o| {
        let s = o.sequence.as_ref().unwrap();
        let (frames, drained) = played.unwrap_or((0, true));
        let mut position = 0;
        for m in &s.marks {
            if m.0 <= frames {
                position = m.1;
            }
        }
        let finished = s.buffers.is_empty() && o.due.is_empty() && o.synth.active() == 0 && drained;
        Ok((position, finished))
    })
    .ok()
}
pub(super) fn sequencer_pause(handle: u64, running: bool) {
    let _ = with_sequence(handle, |o| {
        let s = o.sequence.as_mut().unwrap();
        s.running = running;
        if !running {
            notes_off(&mut o.synth);
        }
        Ok(())
    });
}
pub(super) fn sequencer_close(handle: u64) {
    let _ = with_sequence(handle, |o| {
        notes_off(&mut o.synth);
        o.sequence.as_mut().unwrap().buffers.clear();
        o.due.clear();
        Ok(())
    });
    close(handle);
}
#[unsafe(no_mangle)]
pub extern "system" fn midiInGetErrorTextW(code: u32, out: usize, length: u32) -> u32 {
    midiOutGetErrorTextW(code, out, length)
}
#[unsafe(no_mangle)]
pub extern "system" fn midiInGetErrorTextA(code: u32, out: usize, length: u32) -> u32 {
    midiOutGetErrorTextA(code, out, length)
}
