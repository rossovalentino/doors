//! door-audio (P15): the sound service. It is the only process holding the
//! AUDIO role authority over the kernel's polled virtio-sound driver, and
//! mixes every client stream into the device's one output stream (48 kHz,
//! stereo, 16-bit, 10 ms periods). Clients hand PCM over in sections
//! (door.audio QUEUE); the service maps each one read-only in its section
//! window, resamples it linearly, applies the stream and master volume,
//! balance and mono, and keeps about 100 ms queued on the device. A client's
//! streams stop and unmap when its channel goes away.
//!
//! Capture: while any client holds a capture stream the device's input
//! runs; each 10 ms period read from it is copied into every stream's ring
//! (`audio::capture`) that the microphone policy still allows. A policy
//! change that refuses a stream stops it with state REVOKED; losing the
//! input stops them all with state LOST.
#![no_std]
extern crate alloc;
use alloc::{collections::VecDeque, vec::Vec};
use core::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use door_abi::{
    Handle, Status as SysStatus, Transfer,
    platform::{AUDIO_PERIOD_BYTES, Identity, StartupV2, inherited, manifest},
    rights,
};
use door_service_protocol::{
    Code,
    audio::{self, CaptureStatus, CaptureUse, Format, Output, StreamStatus, capture, op},
};
use door_system_services::server::{Handler, Response, Server};
mod clips;
/// The channel system sounds (`PLAY_SOUND`) play on: no client owns them.
const SYSTEM: Handle = Handle::MAX;
/// System sounds playing at once; a newer one replaces the oldest.
const SYSTEM_SOUNDS: usize = 2;

pub const HEAP_BYTES: u64 = 4 * 1024 * 1024;
/// Output frames in one device period (10 ms at 48 kHz).
pub const PERIOD_FRAMES: usize = AUDIO_PERIOD_BYTES / 4;
/// Periods kept queued on the device while something plays.
const TARGET_QUEUED: u8 = 10;
/// Silent periods (40 ms each) written after the last audible period
/// before the device is stopped. A device takes buffers ahead of playing
/// them (QEMU copies them into its own audio buffer; real controllers have
/// a DMA ring and FIFO), and stopping it discards what it holds: without
/// this tail the last few hundred milliseconds of a sound were cut off.
const TAIL_PERIODS: u8 = 15;
/// Periods (40 ms each) the device must hold before the mixer waits for a
/// late client instead of padding its sound with silence.
const SAFE_QUEUED: u8 = 3;
/// Section window slots (`0x7200_0000..0x8000_0000`, 8 MiB each).
const WINDOW: u64 = 0x7200_0000;
const SLOT_BYTES: u64 = 8 * 1024 * 1024;
const SLOTS: usize = 28;
/// Sections one stream may have queued at once.
const SECTIONS_PER_STREAM: usize = 6;
/// Capture streams one client may hold, and in total.
const CAPTURES_PER_CLIENT: usize = 2;
const CAPTURES: usize = 4;
/// Captured periods taken from the device per poll at most (160 ms).
const CAPTURE_BATCH: usize = 16;
/// Running capture with no period for this long has lost its input.
const CAPTURE_SILENT_MS: u64 = 1000;
/// How often a lost device is probed again.
const LOST_PROBE_MS: u64 = 1000;

/// A capture ring's header through atomics: the client maps the same
/// section and reads while the service writes.
struct Ring {
    base: *mut u8,
}
impl Ring {
    fn word(&self, at: usize) -> &AtomicU32 {
        // SAFETY: `base` is a 64-byte aligned mapping of at least HEADER
        // bytes for as long as the ring is used; the field is 4-aligned.
        unsafe { &*(self.base.add(at) as *const AtomicU32) }
    }
    fn count(&self, at: usize) -> &AtomicU64 {
        // SAFETY: as `word`, the field is 8-aligned.
        unsafe { &*(self.base.add(at) as *const AtomicU64) }
    }
    /// A fresh ring the client prepared: magic, a valid capacity that fits
    /// `mapped`, nothing written or read yet.
    fn validate(&self, mapped: usize) -> Option<usize> {
        let capacity = self.word(capture::CAPACITY).load(Ordering::Acquire) as usize;
        (self.word(capture::MAGIC).load(Ordering::Acquire) == capture::MAGIC_VALUE
            && capture::valid_capacity(capacity)
            && capture::HEADER + capacity <= mapped
            && self.count(capture::WRITE).load(Ordering::Acquire) == 0
            && self.count(capture::READ).load(Ordering::Acquire) == 0)
            .then_some(capacity)
    }
    /// Append one period; false (and an overrun counted) when the client
    /// has not made room for it.
    fn push(&self, capacity: usize, period: &[u8]) -> bool {
        let write = self.count(capture::WRITE).load(Ordering::Relaxed);
        let read = self.count(capture::READ).load(Ordering::Acquire);
        let used = write.wrapping_sub(read) as usize;
        if used > capacity || capacity - used < period.len() {
            self.word(capture::OVERRUNS).fetch_add(1, Ordering::Relaxed);
            return false;
        }
        let start = (write % capacity as u64) as usize;
        let first = period.len().min(capacity - start);
        // SAFETY: both ranges lie inside the ring's data area.
        unsafe {
            let data = self.base.add(capture::HEADER);
            core::ptr::copy_nonoverlapping(period.as_ptr(), data.add(start), first);
            core::ptr::copy_nonoverlapping(period.as_ptr().add(first), data, period.len() - first);
        }
        self.count(capture::WRITE)
            .store(write + period.len() as u64, Ordering::Release);
        true
    }
    fn stop(&self, state: u32) {
        self.word(capture::STATE).store(state, Ordering::Release);
    }
}
/// One client's capture stream: its ring, mapped read-write in a slot.
struct Capture {
    channel: Handle,
    id: u32,
    pid: u32,
    manifest: u32,
    desktop: bool,
    /// The display name the client gave (a CaptureUse carries it).
    name: CaptureUse,
    handle: Handle,
    slot: usize,
    mapped: u64,
    capacity: usize,
}
impl Capture {
    fn ring(&self) -> Ring {
        Ring {
            base: (WINDOW + self.slot as u64 * SLOT_BYTES) as *mut u8,
        }
    }
}
/// Windows programs are the Microphone page's "desktop apps".
fn desktop_app(manifest: u64) -> bool {
    matches!(
        manifest,
        manifest::WINDOWS
            | manifest::WINDOWS_BASE
            | manifest::WINDOWS_APPLICATION
            | manifest::WINDOWS_GUI
    )
}
fn millis() -> u64 {
    door_sys::time()
        .unwrap_or(0)
        .saturating_mul(door_abi::platform::MONOTONIC_TICK_MILLIS)
}

/// One mapped section of a stream's PCM.
struct Chunk {
    handle: Handle,
    slot: usize,
    mapped: u64,
    /// The PCM bytes (whole frames), valid while mapped.
    bytes: &'static [u8],
}
struct Stream {
    channel: Handle,
    id: u32,
    format: Format,
    chunks: VecDeque<Chunk>,
    /// Read position in the front chunk: frames << 16 (16-bit fraction).
    position: u64,
    paused: bool,
    volume: u8,
    played: u64,
    queued: u64,
    /// Generated test tone instead of client sections.
    tone: Option<Tone>,
    /// A system sound: the channel that asked for it and its event.
    sound: Option<(Handle, u32)>,
}
/// A sine oscillator (no libm: the recursion only needs cos and sin of the
/// step, from their series).
#[derive(Clone, Copy)]
struct Tone {
    left: u32,
    coefficient: f64,
    y1: f64,
    y2: f64,
}
impl Tone {
    fn new(hz: f64, amplitude: f64, frames: u32) -> Self {
        let w = 2.0 * core::f64::consts::PI * hz / f64::from(audio::RATE);
        let (w2, w3) = (w * w, w * w * w);
        let cos = 1.0 - w2 / 2.0 + w2 * w2 / 24.0 - w2 * w2 * w2 / 720.0;
        let sin = w - w3 / 6.0 + w3 * w2 / 120.0 - w3 * w2 * w2 / 5040.0;
        Self {
            left: frames,
            coefficient: 2.0 * cos,
            y1: 0.0,
            y2: -amplitude * sin,
        }
    }
    fn next(&mut self) -> i32 {
        let y = self.coefficient * self.y1 - self.y2;
        self.y2 = self.y1;
        self.y1 = y;
        self.left = self.left.saturating_sub(1);
        (y * 32767.0) as i32
    }
}

/// Sample `index` (frame, channel) of `bytes` in `format`, scaled to 16 bits.
fn sample(bytes: &[u8], format: Format, frame: usize, channel: usize) -> i32 {
    let width = usize::from(format.bits / 8);
    let ch = channel.min(usize::from(format.channels) - 1);
    let at = frame * format.frame() + ch * width;
    let Some(s) = bytes.get(at..at + width) else {
        return 0;
    };
    match width {
        1 => (i32::from(s[0]) - 128) << 8,
        2 => i32::from(i16::from_le_bytes([s[0], s[1]])),
        3 => i32::from_le_bytes([0, s[0], s[1], s[2]]) >> 16,
        _ => i32::from_le_bytes([s[0], s[1], s[2], s[3]]) >> 16,
    }
}
impl Stream {
    fn frames_in_front(&self) -> usize {
        self.chunks
            .front()
            .map_or(0, |c| c.bytes.len() / self.format.frame())
    }
    fn audible(&self) -> bool {
        !self.paused && (self.tone.is_some_and(|t| t.left > 0) || !self.chunks.is_empty())
    }
    /// Mix one period into `mix` (stereo i32 pairs) at `gain` (0..=65536);
    /// returns the chunks it finished so the caller unmaps them.
    /// Whether this stream has a whole output period buffered (or makes no
    /// sound now): mixing it sooner would pad its sound with silence.
    fn has_period(&self) -> bool {
        // A client at its section cap cannot send more until this mixes.
        if !self.audible() || self.tone.is_some() || self.chunks.len() >= SECTIONS_PER_STREAM {
            return true;
        }
        let input: u64 = self
            .chunks
            .iter()
            .map(|c| (c.bytes.len() / self.format.frame()) as u64)
            .sum();
        // Output frames the buffered input covers at this stream's rate.
        let remaining = (input << 16).saturating_sub(self.position);
        let needed =
            (PERIOD_FRAMES as u64) * ((u64::from(self.format.rate) << 16) / u64::from(audio::RATE));
        remaining >= needed
    }
    fn mix_into(&mut self, mix: &mut [[i32; 2]], gain: i64) -> Vec<Chunk> {
        let mut done = Vec::new();
        if self.paused {
            return done;
        }
        if let Some(tone) = self.tone.as_mut() {
            for frame in mix.iter_mut() {
                if tone.left == 0 {
                    break;
                }
                let v = ((i64::from(tone.next()) * gain) >> 16) as i32;
                frame[0] += v;
                frame[1] += v;
            }
            return done;
        }
        let step = (u64::from(self.format.rate) << 16) / u64::from(audio::RATE);
        for frame in mix.iter_mut() {
            // Chunks this frame has moved past are finished first: the
            // frame takes its sample from the chunk the position is in, so
            // a section boundary inside a period costs no output frame.
            while let Some(front) = self.chunks.front() {
                let frames = front.bytes.len() / self.format.frame();
                if ((self.position >> 16) as usize) < frames {
                    break;
                }
                self.position -= (frames as u64) << 16;
                done.extend(self.chunks.pop_front());
            }
            let Some(front) = self.chunks.front() else {
                break;
            };
            let frames = front.bytes.len() / self.format.frame();
            let index = (self.position >> 16) as usize;
            let frac = (self.position & 0xFFFF) as i64;
            for (ch, out) in frame.iter_mut().enumerate() {
                let a = sample(front.bytes, self.format, index, ch);
                // The last frame of a chunk interpolates towards the next
                // chunk's first.
                let b = if index + 1 < frames {
                    sample(front.bytes, self.format, index + 1, ch)
                } else if let Some(next) = self.chunks.get(1) {
                    sample(next.bytes, self.format, 0, ch)
                } else {
                    a
                };
                let v = i64::from(a) + ((i64::from(b - a) * frac) >> 16);
                *out += ((v * gain) >> 16) as i32;
            }
            self.position += step;
            let advanced = ((self.position >> 16) as usize).min(frames) - index.min(frames);
            self.played += advanced as u64;
            self.queued = self.queued.saturating_sub(advanced as u64);
        }
        if self.frames_in_front() > 0 && (self.position >> 16) as usize >= self.frames_in_front() {
            let frames = self.frames_in_front() as u64;
            self.position -= frames << 16;
            done.extend(self.chunks.pop_front());
        }
        done
    }
}

/// Master volume as a gain (0..=65536): squared, so the slider's middle is
/// about -12 dB, close to Windows' audio taper.
fn master_gain(output: Output) -> i64 {
    if output.muted {
        return 0;
    }
    let v = i64::from(output.volume);
    v * v * 65536 / 10_000
}
/// Finish a mixed period: balance, mono, clipping, interleaved S16LE.
fn finish(mix: &[[i32; 2]], output: Output, out: &mut [u8]) {
    let b = i64::from(output.balance);
    let (left, right) = (
        if b > 0 { 100 - b } else { 100 },
        if b < 0 { 100 + b } else { 100 },
    );
    for (i, frame) in mix.iter().enumerate() {
        let (mut l, mut r) = (i64::from(frame[0]), i64::from(frame[1]));
        if output.mono {
            let m = (l + r) / 2;
            (l, r) = (m, m);
        }
        let l = (l * left / 100).clamp(-32768, 32767) as i16;
        let r = (r * right / 100).clamp(-32768, 32767) as i16;
        out[i * 4..i * 4 + 2].copy_from_slice(&l.to_le_bytes());
        out[i * 4 + 2..i * 4 + 4].copy_from_slice(&r.to_le_bytes());
    }
}

pub struct Audio {
    device: Option<Handle>,
    /// The device answers: false after it failed (removed, reset), until a
    /// probe finds it again.
    present: bool,
    /// When to probe a lost device next (milliseconds).
    probe_at: u64,
    output: Output,
    streams: Vec<Stream>,
    slots: [bool; SLOTS],
    next_id: u32,
    running: bool,
    /// Silent periods still to write after the last audible one before the
    /// device is stopped (`TAIL_PERIODS`).
    tail: u8,
    played: u64,
    underruns: u64,
    /// When each client last played a system sound (rate limit).
    last_sound: Vec<(Handle, u64)>,
    /// The last period went to the device with nothing audible (silence
    /// keeps the stream fed until the queue drains).
    period: Vec<u8>,
    captures: Vec<Capture>,
    /// The device has an input; the service started it.
    capture_present: bool,
    pub capturing: bool,
    /// The microphone policy word (`audio::capture`).
    capture_policy: u32,
    /// The microphone level applied to captured periods.
    input: audio::Input,
    /// Recent microphone users, most recent first.
    recent: Vec<CaptureUse>,
    /// When the input last returned a period (or started).
    last_capture: u64,
}
impl Audio {
    pub fn new(device: Option<Handle>) -> Self {
        Self {
            device,
            present: device.is_some_and(|d| door_sys::audio_info(d).is_ok()),
            probe_at: 0,
            output: Output::default(),
            streams: Vec::new(),
            slots: [false; SLOTS],
            next_id: 1,
            running: false,
            tail: 0,
            played: 0,
            underruns: 0,
            last_sound: Vec::new(),
            period: alloc::vec![0; AUDIO_PERIOD_BYTES],
            captures: Vec::new(),
            capture_present: device
                .and_then(|d| door_sys::audio_info(d).ok())
                .is_some_and(|i| i.capture != door_abi::platform::Capture::Absent),
            capturing: false,
            // Closed until the shell hands over the signed-in user's
            // Privacy & security > Microphone settings.
            capture_policy: capture::ACCESS_OFF,
            input: audio::Input::default(),
            recent: Vec::new(),
            last_capture: 0,
        }
    }
    fn status(&self) -> audio::Status {
        audio::Status {
            present: self.usable(),
            running: self.running,
            output: self.output,
            streams: self.streams.len() as u8,
            rate: audio::RATE,
            channels: audio::CHANNELS,
            bits: 16,
            played: self.played,
            underruns: self.underruns,
            capture: self.capture_present,
            capturing: self.captures.len() as u8,
            input: self.input,
        }
    }
    fn capture_status(&self) -> CaptureStatus {
        let mut status = CaptureStatus {
            present: self.capture_present,
            policy: self.capture_policy,
            now: millis(),
            used: self.recent.len().min(audio::RECENT) as u8,
            ..Default::default()
        };
        for (slot, entry) in status.recent.iter_mut().zip(&self.recent) {
            *slot = *entry;
        }
        status
    }
    /// Note `pid`'s use of the microphone now (active or ended).
    fn note_use(&mut self, pid: u32, manifest: u32, desktop: bool, active: bool, name: &str) {
        let now = millis();
        self.recent.retain(|u| u.pid != pid);
        self.recent.insert(
            0,
            CaptureUse {
                pid,
                manifest,
                last: now,
                active,
                desktop,
                ..Default::default()
            }
            .with_name(name),
        );
        self.recent.truncate(audio::RECENT);
    }
    fn close_capture(&mut self, index: usize, state: Option<u32>) {
        let c = self.captures.remove(index);
        if let Some(state) = state {
            c.ring().stop(state);
        }
        let still = self.captures.iter().any(|o| o.pid == c.pid);
        let name = c.name;
        self.note_use(c.pid, c.manifest, c.desktop, still, name.name());
        // SAFETY: no reference into the ring outlives this call.
        let _ = unsafe { door_sys::section_unmap(WINDOW + c.slot as u64 * SLOT_BYTES, c.mapped) };
        let _ = door_sys::close(c.handle);
        self.slots[c.slot] = false;
    }
    /// A new policy: streams it refuses stop with REVOKED.
    fn set_capture_policy(&mut self, policy: u32) {
        self.capture_policy = policy;
        while let Some(index) = self
            .captures
            .iter()
            .position(|c| !capture::allows(policy, c.desktop))
        {
            self.close_capture(index, Some(capture::REVOKED));
        }
    }
    /// Run the input while streams want it and copy what it captured into
    /// their rings.
    fn capture_poll(&mut self, device: Handle) {
        use door_abi::platform::{AUDIO_CAPTURE_START, AUDIO_CAPTURE_STOP};
        if self.captures.is_empty() {
            if self.capturing {
                let _ = door_sys::audio_control(device, AUDIO_CAPTURE_STOP);
                self.capturing = false;
            }
            return;
        }
        if !self.capturing {
            if door_sys::audio_control(device, AUDIO_CAPTURE_START).is_err() {
                self.capture_lost();
                return;
            }
            self.capturing = true;
            self.last_capture = millis();
            door_rt::diagnostic(format_args!(
                "DOOR_P15 AUDIO CAPTURE started streams={}
",
                self.captures.len()
            ));
        }
        let mut period = [0u8; AUDIO_PERIOD_BYTES];
        for _ in 0..CAPTURE_BATCH {
            match door_sys::audio_read(device, &mut period) {
                Ok(true) => {
                    self.last_capture = millis();
                    if self.input != audio::Input::default() {
                        for s in period.chunks_exact_mut(2) {
                            let v = self.input.apply(i16::from_le_bytes([s[0], s[1]]));
                            s.copy_from_slice(&v.to_le_bytes());
                        }
                    }
                    for c in &self.captures {
                        c.ring().push(c.capacity, &period);
                    }
                }
                Ok(false) => {
                    // An input that returns nothing for a second is gone (a
                    // host backend without capture never answers).
                    if millis().saturating_sub(self.last_capture) > CAPTURE_SILENT_MS {
                        self.capture_lost();
                        return;
                    }
                    break;
                }
                Err(_) => {
                    self.capture_lost();
                    return;
                }
            }
        }
    }
    /// The device is there and answering.
    fn usable(&self) -> bool {
        self.device.is_some() && self.present
    }
    /// The output failed: every playback stream ends (its client sees it
    /// gone: winmm hands its buffers back, WASAPI answers
    /// AUDCLNT_E_DEVICE_INVALIDATED from the status), capture with it, and
    /// the device is probed again every second.
    fn output_lost(&mut self) {
        door_rt::diagnostic(format_args!(
            "DOOR_P15 AUDIO OUTPUT lost streams={}\n",
            self.streams.len()
        ));
        while !self.streams.is_empty() {
            self.close_stream(0);
        }
        self.running = false;
        self.present = false;
        self.probe_at = millis() + LOST_PROBE_MS;
        if !self.captures.is_empty() || self.capturing {
            self.capture_lost();
        }
    }
    /// A lost device answers again: playback and capture may open.
    fn probe(&mut self, device: Handle) {
        if millis() < self.probe_at {
            return;
        }
        self.probe_at = millis() + LOST_PROBE_MS;
        if let Ok(info) = door_sys::audio_info(device) {
            self.present = true;
            self.capture_present = info.capture != door_abi::platform::Capture::Absent;
            door_rt::diagnostic(format_args!(
                "DOOR_P15 AUDIO OUTPUT restored capture={}\n",
                u8::from(self.capture_present)
            ));
        }
    }
    /// An input that went silent may be back (a microphone plugged in
    /// again): asked when a capture stream opens.
    fn probe_capture(&mut self) {
        if let Some(device) = self
            .device
            .filter(|_| self.present && !self.capture_present)
            && door_sys::audio_info(device)
                .is_ok_and(|i| i.capture != door_abi::platform::Capture::Absent)
        {
            self.capture_present = true;
            self.last_capture = millis();
        }
    }
    /// The input failed: every stream stops with LOST.
    fn capture_lost(&mut self) {
        door_rt::diagnostic(format_args!(
            "DOOR_P15 AUDIO CAPTURE lost streams={}
",
            self.captures.len()
        ));
        while !self.captures.is_empty() {
            self.close_capture(0, Some(capture::LOST));
        }
        if self.capturing
            && let Some(device) = self.device
        {
            let _ = door_sys::audio_control(device, door_abi::platform::AUDIO_CAPTURE_STOP);
        }
        self.capturing = false;
        self.capture_present = false;
    }
    fn release(&mut self, chunk: Chunk) {
        if chunk.slot >= SLOTS {
            // A built-in system sound: nothing was mapped.
            return;
        }
        // SAFETY: the chunk's mapping is unreferenced once it leaves the
        // stream (no slice into it survives the mixing call).
        let _ = unsafe {
            door_sys::section_unmap(WINDOW + chunk.slot as u64 * SLOT_BYTES, chunk.mapped)
        };
        let _ = door_sys::close(chunk.handle);
        self.slots[chunk.slot] = false;
    }
    fn close_stream(&mut self, index: usize) {
        let mut s = self.streams.remove(index);
        while let Some(chunk) = s.chunks.pop_front() {
            self.release(chunk);
        }
    }
    /// Mix one period of every audible stream into `self.period`.
    fn mix(&mut self) {
        let mut mix = [[0i32; 2]; PERIOD_FRAMES];
        let master = master_gain(self.output);
        let mut finished = Vec::new();
        for s in &mut self.streams {
            let gain = master * i64::from(s.volume) / 100;
            finished.extend(s.mix_into(&mut mix, gain));
        }
        for chunk in finished {
            self.release(chunk);
        }
        self.streams.retain(|s| {
            s.tone.is_none_or(|t| t.left > 0) && !(s.channel == SYSTEM && s.chunks.is_empty())
        });
        finish(&mix, self.output, &mut self.period);
    }
    /// Keep the device fed; true when anything was written.
    pub fn poll(&mut self) -> bool {
        let Some(device) = self.device else {
            return false;
        };
        if !self.present {
            self.probe(device);
            return false;
        }
        if self.capture_present {
            self.capture_poll(device);
        }
        let Ok(info) = door_sys::audio_info(device) else {
            self.output_lost();
            return true;
        };
        let audible = self.streams.iter().any(Stream::audible);
        let mut queued = info.queued;
        let mut wrote = false;
        if self.running && audible && queued == 0 {
            self.underruns += 1;
        }
        if audible {
            self.tail = TAIL_PERIODS;
        }
        // After the last audible period the device is fed silence for the
        // tail, so what it already holds plays out before it is stopped.
        while (audible || (self.running && self.tail > 0)) && queued < TARGET_QUEUED {
            if !audible {
                self.tail -= 1;
            }
            // While the device has a safe margin queued, wait for every
            // playing stream to have a whole period: a client a little late
            // (not scheduled for a few tens of milliseconds) then costs
            // nothing instead of a gap of silence in the middle of its sound.
            // Short of the margin, mix what there is (the end of a sound,
            // or a client that really fell behind).
            if audible && queued >= SAFE_QUEUED && !self.streams.iter().all(Stream::has_period) {
                break;
            }
            self.mix();
            match door_sys::audio_write(device, &self.period) {
                Ok(_) => (),
                // A full queue waits for the next poll; anything else is
                // the device failing.
                Err(door_abi::Status::WouldBlock) => break,
                Err(_) => {
                    self.output_lost();
                    return true;
                }
            }
            queued += 1;
            wrote = true;
        }
        if audible && !self.running && queued >= 2 {
            self.running = door_sys::audio_control(device, door_abi::platform::AUDIO_START).is_ok();
        } else if !audible
            && self.running
            && self.tail == 0
            && queued == 0
            && door_sys::audio_control(device, door_abi::platform::AUDIO_STOP).is_ok()
        {
            self.running = false;
        }
        self.played = info.played;
        wrote
    }
    fn stream(&self, channel: Handle, id: u32) -> Option<usize> {
        self.streams
            .iter()
            .position(|s| s.channel == channel && s.id == id)
    }
}
fn word(body: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(body.get(at..at + 4)?.try_into().ok()?))
}
impl Handler for Audio {
    fn request(
        &mut self,
        channel: Handle,
        peer: Identity,
        opcode: u32,
        body: &[u8],
        transfers: &[Transfer],
    ) -> Response {
        // The display service (quick settings) and Settings own the master
        // output; any app may play, and change the volume and mute as
        // Windows programs may (IAudioEndpointVolume, the mixer API) — the
        // shell adopts and persists such a change. Mono audio and balance
        // are the shell's alone.
        let shell = matches!(peer.manifest, manifest::WINDOW | manifest::WORKSPACE);
        match opcode {
            op::STATUS if body.is_empty() => Response::bytes(&self.status().encode()),
            op::SET_OUTPUT => {
                let Some(output) = Output::decode(body) else {
                    return Response::error(Code::InvalidRequest);
                };
                if !shell
                    && (output.mono != self.output.mono || output.balance != self.output.balance)
                {
                    return Response::error(Code::Denied);
                }
                if output != self.output {
                    self.output = output;
                    door_rt::diagnostic(format_args!(
                        "DOOR_P15 AUDIO OUTPUT volume={} muted={} mono={} balance={}\n",
                        output.volume,
                        u8::from(output.muted),
                        u8::from(output.mono),
                        output.balance
                    ));
                }
                Response::empty()
            }
            op::SET_INPUT => {
                let Some(input) = audio::Input::decode(body) else {
                    return Response::error(Code::InvalidRequest);
                };
                if input != self.input {
                    self.input = input;
                    door_rt::diagnostic(format_args!(
                        "DOOR_P15 AUDIO INPUT volume={} muted={}\n",
                        input.volume,
                        u8::from(input.muted)
                    ));
                }
                Response::empty()
            }
            op::OPEN | op::TEST_TONE => {
                if !self.usable() {
                    return Response::error(Code::Unsupported);
                }
                let format = if opcode == op::OPEN {
                    match Format::decode(body) {
                        Some(f) => f,
                        None => return Response::error(Code::InvalidRequest),
                    }
                } else if shell && body.is_empty() {
                    Format {
                        rate: audio::RATE,
                        channels: 2,
                        bits: 16,
                    }
                } else {
                    return Response::error(Code::Denied);
                };
                if self.streams.len() >= audio::STREAMS
                    || self.streams.iter().filter(|s| s.channel == channel).count()
                        >= audio::STREAMS_PER_CLIENT
                {
                    return Response::error(Code::Resources);
                }
                let id = self.next_id;
                self.next_id += 1;
                self.streams.push(Stream {
                    channel,
                    id,
                    format,
                    chunks: VecDeque::new(),
                    position: 0,
                    paused: false,
                    volume: 100,
                    played: 0,
                    queued: 0,
                    tone: (opcode == op::TEST_TONE).then(|| Tone::new(440.0, 0.3, audio::RATE)),
                    sound: None,
                });
                if opcode == op::TEST_TONE {
                    door_rt::diagnostic(format_args!("DOOR_P15 AUDIO TEST_TONE id={id}\n"));
                }
                Response::scalar(u64::from(id))
            }
            op::PLAY_SOUND => {
                let Some(event) = word(body, 0).filter(|_| body.len() == 4) else {
                    return Response::error(Code::InvalidRequest);
                };
                // The shell plays every event; Clock its alarm; other apps
                // only the MessageBeep sounds (never UAC, sign-in or device
                // cues), at most one every 100 ms per client.
                use audio::sound;
                let allowed = shell
                    || (peer.manifest == manifest::CLOCK
                        && matches!(event, sound::ALARM | sound::NOTIFICATION))
                    || matches!(
                        event,
                        sound::DEFAULT_BEEP
                            | sound::ASTERISK
                            | sound::EXCLAMATION
                            | sound::CRITICAL_STOP
                    );
                if !allowed {
                    return Response::error(Code::Denied);
                }
                let now = door_sys::time().unwrap_or(0);
                if !shell {
                    if let Some(last) = self.last_sound.iter_mut().find(|(c, _)| *c == channel) {
                        if now.saturating_sub(last.1)
                            < 100 / door_abi::platform::MONOTONIC_TICK_MILLIS
                        {
                            return Response::error(Code::Resources);
                        }
                        last.1 = now;
                    } else {
                        self.last_sound.push((channel, now));
                    }
                }
                if !self.usable() {
                    return Response::error(Code::Unsupported);
                }
                let Some((format, pcm)) = clips::file(event).and_then(clips::parse) else {
                    return Response::error(Code::NotFound);
                };
                let playing: Vec<usize> = (0..self.streams.len())
                    .filter(|i| self.streams[*i].channel == SYSTEM)
                    .collect();
                if playing.len() >= SYSTEM_SOUNDS {
                    // Replace the caller's own sound first, else the oldest
                    // that is not an alarm; a ringing alarm is never cut off
                    // by an app.
                    let victim = playing
                        .iter()
                        .copied()
                        .find(|i| self.streams[*i].sound.is_some_and(|(c, _)| c == channel))
                        .or_else(|| {
                            playing.iter().copied().find(|i| {
                                self.streams[*i]
                                    .sound
                                    .is_none_or(|(_, e)| e != sound::ALARM)
                            })
                        });
                    match victim {
                        Some(i) => self.close_stream(i),
                        None => return Response::error(Code::Resources),
                    }
                }
                if self.streams.len() >= audio::STREAMS {
                    return Response::error(Code::Resources);
                }
                let id = self.next_id;
                self.next_id += 1;
                let mut chunks = VecDeque::new();
                chunks.push_back(Chunk {
                    handle: 0,
                    slot: SLOTS,
                    mapped: 0,
                    bytes: pcm,
                });
                self.streams.push(Stream {
                    channel: SYSTEM,
                    id,
                    format,
                    chunks,
                    position: 0,
                    paused: false,
                    volume: 100,
                    played: 0,
                    queued: (pcm.len() / format.frame()) as u64,
                    tone: None,
                    sound: Some((channel, event)),
                });
                door_rt::diagnostic(format_args!(
                    "DOOR_P15 AUDIO SOUND event={event} rate={} frames={}\n",
                    format.rate,
                    pcm.len() / format.frame()
                ));
                Response::scalar(u64::from(id))
            }
            op::QUEUE => {
                let (Some(id), Some(len)) = (word(body, 0), word(body, 4)) else {
                    return Response::error(Code::InvalidRequest);
                };
                let [section] = transfers else {
                    return Response::error(Code::InvalidRequest);
                };
                let section = section.handle;
                let refuse = |code| {
                    let _ = door_sys::close(section);
                    Response::error(code)
                };
                let Some(index) = self.stream(channel, id) else {
                    return refuse(Code::NotFound);
                };
                let len = len as usize;
                let frame = self.streams[index].format.frame();
                if len == 0 || len > audio::MAX_SECTION || !len.is_multiple_of(frame) {
                    return refuse(Code::InvalidRequest);
                }
                if self.streams[index].chunks.len() >= SECTIONS_PER_STREAM {
                    return refuse(Code::WouldBlock);
                }
                let Some(slot) = self.slots.iter().position(|used| !used) else {
                    return refuse(Code::Resources);
                };
                let address = WINDOW + slot as u64 * SLOT_BYTES;
                // SAFETY: the slot's range is reserved for this view and no
                // Rust reference into it exists until the map succeeds.
                let mapped = match unsafe { door_sys::section_map(section, address, rights::READ) }
                {
                    Ok(bytes) if bytes >= len as u64 && bytes <= SLOT_BYTES => bytes,
                    Ok(bytes) => {
                        let _ = unsafe { door_sys::section_unmap(address, bytes) };
                        return refuse(Code::InvalidRequest);
                    }
                    Err(_) => return refuse(Code::InvalidRequest),
                };
                self.slots[slot] = true;
                // SAFETY: the read-only view stays mapped until `release`.
                let bytes = unsafe { core::slice::from_raw_parts(address as *const u8, len) };
                let s = &mut self.streams[index];
                s.queued += (len / frame) as u64;
                s.chunks.push_back(Chunk {
                    handle: section,
                    slot,
                    mapped,
                    bytes,
                });
                Response::scalar(s.queued)
            }
            op::STREAM => {
                let Some(index) = word(body, 0).and_then(|id| self.stream(channel, id)) else {
                    return Response::error(Code::NotFound);
                };
                let s = &self.streams[index];
                Response::bytes(
                    &StreamStatus {
                        paused: s.paused,
                        drained: s.chunks.is_empty() && s.tone.is_none_or(|t| t.left == 0),
                        volume: s.volume,
                        played: s.played,
                        queued: s.queued,
                    }
                    .encode(),
                )
            }
            op::PAUSE | op::STREAM_VOLUME => {
                let (Some(index), Some(&value)) = (
                    word(body, 0).and_then(|id| self.stream(channel, id)),
                    body.get(4),
                ) else {
                    return Response::error(Code::InvalidRequest);
                };
                if body.len() != 5 || (opcode == op::PAUSE && value > 1) || value > 100 {
                    return Response::error(Code::InvalidRequest);
                }
                let s = &mut self.streams[index];
                if opcode == op::PAUSE {
                    s.paused = value == 1;
                } else {
                    s.volume = value;
                }
                Response::empty()
            }
            op::CLOSE => {
                let Some(index) = word(body, 0).and_then(|id| self.stream(channel, id)) else {
                    return Response::error(Code::NotFound);
                };
                self.close_stream(index);
                Response::empty()
            }
            op::CAPTURE_OPEN => {
                let [section] = transfers else {
                    return Response::error(Code::InvalidRequest);
                };
                let section = section.handle;
                let refuse = |code| {
                    let _ = door_sys::close(section);
                    Response::error(code)
                };
                let Some(name) = core::str::from_utf8(body)
                    .ok()
                    .filter(|n| n.len() <= audio::NAME_BYTES)
                else {
                    return refuse(Code::InvalidRequest);
                };
                self.probe_capture();
                if !self.usable() || !self.capture_present {
                    return refuse(Code::Unsupported);
                }
                let desktop = desktop_app(peer.manifest);
                if !capture::allows(self.capture_policy, desktop) {
                    return refuse(Code::Denied);
                }
                if self.captures.len() >= CAPTURES
                    || self
                        .captures
                        .iter()
                        .filter(|c| c.channel == channel)
                        .count()
                        >= CAPTURES_PER_CLIENT
                {
                    return refuse(Code::Resources);
                }
                let Some(slot) = self.slots.iter().position(|used| !used) else {
                    return refuse(Code::Resources);
                };
                let address = WINDOW + slot as u64 * SLOT_BYTES;
                // SAFETY: the slot's range is reserved for this view.
                let mapped = match unsafe {
                    door_sys::section_map(section, address, rights::READ | rights::WRITE)
                } {
                    Ok(bytes) if bytes <= SLOT_BYTES => bytes,
                    Ok(bytes) => {
                        let _ = unsafe { door_sys::section_unmap(address, bytes) };
                        return refuse(Code::InvalidRequest);
                    }
                    Err(_) => return refuse(Code::InvalidRequest),
                };
                let ring = Ring {
                    base: address as *mut u8,
                };
                let Some(capacity) = ring.validate(mapped as usize) else {
                    let _ = unsafe { door_sys::section_unmap(address, mapped) };
                    return refuse(Code::InvalidRequest);
                };
                ring.stop(capture::RUNNING);
                self.slots[slot] = true;
                let id = self.next_id;
                self.next_id += 1;
                let (pid, manifest) = (peer.pid as u32, peer.manifest as u32);
                let label = CaptureUse::default().with_name(name);
                self.captures.push(Capture {
                    channel,
                    id,
                    pid,
                    manifest,
                    desktop,
                    name: label,
                    handle: section,
                    slot,
                    mapped,
                    capacity,
                });
                self.note_use(pid, manifest, desktop, true, name);
                Response::scalar(u64::from(id))
            }
            op::CAPTURE_CLOSE => {
                let Some(index) = word(body, 0).filter(|_| body.len() == 4).and_then(|id| {
                    self.captures
                        .iter()
                        .position(|c| c.channel == channel && c.id == id)
                }) else {
                    return Response::error(Code::NotFound);
                };
                self.close_capture(index, None);
                Response::empty()
            }
            op::CAPTURE_POLICY => {
                if !shell {
                    return Response::error(Code::Denied);
                }
                let Some(policy) =
                    word(body, 0).filter(|p| body.len() == 4 && p & !capture::POLICY_ALL == 0)
                else {
                    return Response::error(Code::InvalidRequest);
                };
                if policy != self.capture_policy {
                    door_rt::diagnostic(format_args!(
                        "DOOR_P15 AUDIO CAPTURE policy={policy}
"
                    ));
                    self.set_capture_policy(policy);
                }
                Response::empty()
            }
            op::CAPTURE_STATUS if body.is_empty() => {
                Response::bytes(&self.capture_status().encode())
            }
            _ => Response::error(Code::InvalidRequest),
        }
    }
    fn disconnected(&mut self, channel: Handle) {
        self.last_sound.retain(|(c, _)| *c != channel);
        while let Some(index) = self.captures.iter().position(|c| c.channel == channel) {
            self.close_capture(index, None);
        }
        while let Some(index) = self.streams.iter().position(|s| s.channel == channel) {
            self.close_stream(index);
        }
    }
}

pub fn run(startup: StartupV2) -> i64 {
    let result = (|| -> Result<(), SysStatus> {
        let mut server = Server::new(&startup)?;
        let handle = startup.base.inherited[inherited::AUTHORITY].handle;
        let info = door_sys::audio_info(handle);
        door_rt::diagnostic(format_args!(
            "DOOR_P15 AUDIO DEVICE present={} rate={} channels={} period={}\n",
            u8::from(info.is_ok()),
            info.map_or(0, |i| i.rate),
            info.map_or(0, |i| i.channels),
            AUDIO_PERIOD_BYTES
        ));
        let mut audio = Audio::new(info.is_ok().then_some(handle));
        server.ready(manifest::AUDIO)?;
        while server.poll(&mut audio)? {
            if audio.poll() || audio.running || audio.capturing {
                server.note_progress();
            }
            server.idle_backoff()?;
        }
        Ok(())
    })();
    match result {
        Ok(()) => 0,
        Err(status) => {
            door_rt::diagnostic(format_args!("DOOR_P15 AUDIO FAILED status={status:?}\n"));
            -152
        }
    }
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    #[test]
    fn capture_rings_validate_wrap_and_count_overruns() {
        let capacity = capture::MIN_CAPACITY;
        let mut memory = alloc::vec![0u64; (capture::HEADER + capacity) / 8];
        let ring = Ring {
            base: memory.as_mut_ptr() as *mut u8,
        };
        assert_eq!(ring.validate(capture::HEADER + capacity), None, "no magic");
        ring.word(capture::MAGIC)
            .store(capture::MAGIC_VALUE, Ordering::Relaxed);
        ring.word(capture::CAPACITY)
            .store(capacity as u32, Ordering::Relaxed);
        assert_eq!(
            ring.validate(capture::HEADER + capacity - 1),
            None,
            "does not fit the section"
        );
        assert_eq!(ring.validate(capture::HEADER + capacity), Some(capacity));
        let period = [0x11u8; capture::PERIOD_BYTES];
        for _ in 0..capacity / capture::PERIOD_BYTES {
            assert!(ring.push(capacity, &period));
        }
        // Full: the next period is dropped and counted.
        assert!(!ring.push(capacity, &[0x22; capture::PERIOD_BYTES]));
        assert_eq!(ring.word(capture::OVERRUNS).load(Ordering::Relaxed), 1);
        // The client reads one and a half periods; the next push wraps.
        ring.count(capture::READ)
            .store(capture::PERIOD_BYTES as u64 * 3 / 2, Ordering::Relaxed);
        assert!(ring.push(capacity, &[0x33; capture::PERIOD_BYTES]));
        let data = unsafe { core::slice::from_raw_parts(ring.base.add(capture::HEADER), capacity) };
        assert!(data[..capture::PERIOD_BYTES].iter().all(|b| *b == 0x33));
        assert_eq!(
            ring.count(capture::WRITE).load(Ordering::Relaxed),
            (capacity + capture::PERIOD_BYTES) as u64
        );
        assert!(
            !ring.push(capacity, &period),
            "half a period free is not enough"
        );
        // A written ring is not fresh.
        assert_eq!(ring.validate(capture::HEADER + capacity), None);
    }
    #[test]
    fn programs_set_the_microphone_level() {
        let mut audio = Audio::new(None);
        let program = Identity {
            pid: 11,
            manifest: manifest::WINDOWS_APPLICATION,
            ..Default::default()
        };
        let input = audio::Input {
            volume: 40,
            muted: false,
        };
        let r = audio.request(5, program, op::SET_INPUT, &input.encode(), &[]);
        assert_eq!(r.code, Code::Success);
        assert_eq!(audio.status().input, input);
        let r = audio.request(5, program, op::SET_INPUT, &[101, 0], &[]);
        assert_eq!(r.code, Code::InvalidRequest);
    }
    #[test]
    fn programs_set_volume_and_mute_but_not_mono_or_balance() {
        let mut audio = Audio::new(None);
        let program = Identity {
            pid: 11,
            manifest: manifest::WINDOWS_APPLICATION,
            ..Default::default()
        };
        let mut output = audio.output;
        output.volume = 30;
        output.muted = true;
        let r = audio.request(5, program, op::SET_OUTPUT, &output.encode(), &[]);
        assert_eq!(r.code, Code::Success);
        assert_eq!(audio.output, output);
        let mut mono = output;
        mono.mono = !mono.mono;
        let r = audio.request(5, program, op::SET_OUTPUT, &mono.encode(), &[]);
        assert_eq!(r.code, Code::Denied);
        let mut balance = output;
        balance.balance = balance.balance.wrapping_add(10);
        let r = audio.request(5, program, op::SET_OUTPUT, &balance.encode(), &[]);
        assert_eq!(r.code, Code::Denied);
        assert_eq!(audio.output, output);
        let shell = Identity {
            manifest: manifest::WINDOW,
            ..Default::default()
        };
        let r = audio.request(5, shell, op::SET_OUTPUT, &mono.encode(), &[]);
        assert_eq!(r.code, Code::Success);
    }
    #[test]
    fn only_the_shell_sets_the_capture_policy_and_status_reports_it() {
        let mut audio = Audio::new(None);
        let windows = Identity {
            pid: 11,
            manifest: manifest::WINDOWS_APPLICATION,
            ..Default::default()
        };
        // Without a device: Unsupported, never Denied (the page can tell).
        let r = audio.request(5, windows, op::CAPTURE_OPEN, &[], &[]);
        assert_eq!(r.code, Code::InvalidRequest, "no section transferred");
        let r = audio.request(
            5,
            windows,
            op::CAPTURE_POLICY,
            &capture::DESKTOP_OFF.to_le_bytes(),
            &[],
        );
        assert_eq!(
            r.code,
            Code::Denied,
            "only the shell and Settings set the policy"
        );
        let shell = Identity {
            manifest: manifest::WORKSPACE,
            ..Default::default()
        };
        let r = audio.request(5, shell, op::CAPTURE_POLICY, &8u32.to_le_bytes(), &[]);
        assert_eq!(r.code, Code::InvalidRequest);
        let r = audio.request(
            5,
            shell,
            op::CAPTURE_POLICY,
            &capture::DESKTOP_OFF.to_le_bytes(),
            &[],
        );
        assert_eq!(r.code, Code::Success);
        let status = CaptureStatus::decode(
            &audio.request(5, windows, op::CAPTURE_STATUS, &[], &[]).body
                [..audio::CAPTURE_STATUS_BYTES],
        )
        .unwrap();
        assert_eq!(
            (status.present, status.policy, status.used),
            (false, capture::DESKTOP_OFF, 0)
        );
        assert!(desktop_app(manifest::WINDOWS_GUI) && !desktop_app(manifest::WORKSPACE));
    }
    fn stream(format: Format, pcm: &'static [u8]) -> Stream {
        let mut chunks = VecDeque::new();
        chunks.push_back(Chunk {
            handle: 0,
            slot: 0,
            mapped: 0,
            bytes: pcm,
        });
        Stream {
            channel: 1,
            id: 1,
            format,
            queued: (pcm.len() / format.frame()) as u64,
            chunks,
            position: 0,
            paused: false,
            volume: 100,
            played: 0,
            tone: None,
            sound: None,
        }
    }
    fn pcm16(samples: &[i16]) -> &'static [u8] {
        let bytes: Vec<u8> = samples.iter().flat_map(|s| s.to_le_bytes()).collect();
        std::boxed::Box::leak(bytes.into_boxed_slice())
    }
    #[test]
    fn a_period_is_ready_only_when_buffered_or_capped() {
        let f = Format {
            rate: 48_000,
            channels: 2,
            bits: 16,
        };
        // Half a period: mixing it now would pad the sound with silence.
        let half = stream(f, pcm16(&alloc::vec![1i16; PERIOD_FRAMES]));
        assert!(!half.has_period());
        let full = stream(f, pcm16(&alloc::vec![1i16; PERIOD_FRAMES * 2]));
        assert!(full.has_period());
        // 24 kHz input covers twice the output frames.
        let low = Format { rate: 24_000, ..f };
        assert!(stream(low, pcm16(&alloc::vec![1i16; PERIOD_FRAMES])).has_period());
        // A client at its section cap cannot add more: ready.
        let mut capped = stream(f, pcm16(&[1, 1]));
        for _ in 1..SECTIONS_PER_STREAM {
            capped.chunks.push_back(Chunk {
                handle: 0,
                slot: 0,
                mapped: 0,
                bytes: pcm16(&[1, 1]),
            });
        }
        assert!(capped.has_period());
        // Paused or silent streams never hold the mix back.
        let mut paused = stream(f, pcm16(&[1, 1]));
        paused.paused = true;
        assert!(paused.has_period());
    }
    #[test]
    fn same_rate_stereo_passes_through_at_full_gain() {
        let f = Format {
            rate: 48_000,
            channels: 2,
            bits: 16,
        };
        let samples: Vec<i16> = (0..PERIOD_FRAMES * 2)
            .map(|i| (i as i16) * 3 - 900)
            .collect();
        let mut s = stream(f, pcm16(&samples));
        let mut mix = [[0i32; 2]; PERIOD_FRAMES];
        let done = s.mix_into(&mut mix, 65536);
        assert_eq!(done.len(), 1, "the chunk ends with the period");
        for (i, frame) in mix.iter().enumerate() {
            assert_eq!(frame[0], i32::from(samples[2 * i]));
            assert_eq!(frame[1], i32::from(samples[2 * i + 1]));
        }
        assert_eq!((s.played, s.queued), (PERIOD_FRAMES as u64, 0));
    }
    #[test]
    fn section_boundaries_inside_a_period_lose_no_frame() {
        // Three sections of 100, 37 and 343 frames make one 480-frame
        // period: the mix is the samples in order, sample for sample.
        let f = Format {
            rate: 48_000,
            channels: 2,
            bits: 16,
        };
        let samples: Vec<i16> = (0..PERIOD_FRAMES * 2).map(|i| i as i16 - 400).collect();
        let mut s = stream(f, pcm16(&samples[..200]));
        for range in [200..274, 274..PERIOD_FRAMES * 2] {
            let bytes = pcm16(&samples[range]);
            s.queued += (bytes.len() / f.frame()) as u64;
            s.chunks.push_back(Chunk {
                handle: 0,
                slot: 0,
                mapped: 0,
                bytes,
            });
        }
        let mut mix = [[0i32; 2]; PERIOD_FRAMES];
        let done = s.mix_into(&mut mix, 65536);
        assert_eq!(done.len(), 3);
        for (i, frame) in mix.iter().enumerate() {
            assert_eq!(
                *frame,
                [i32::from(samples[2 * i]), i32::from(samples[2 * i + 1])],
                "frame {i}"
            );
        }
        assert_eq!((s.played, s.queued), (PERIOD_FRAMES as u64, 0));
    }
    #[test]
    fn half_rate_mono_is_doubled_interpolated_and_spread_to_both_channels() {
        let f = Format {
            rate: 24_000,
            channels: 1,
            bits: 16,
        };
        let mut s = stream(f, pcm16(&[0, 1000, 2000, 3000]));
        let mut mix = [[0i32; 2]; 8];
        s.mix_into(&mut mix, 65536);
        assert_eq!(mix[0], [0, 0]);
        assert_eq!(mix[1], [500, 500], "midpoint between two source frames");
        assert_eq!(mix[2], [1000, 1000]);
        assert_eq!(s.played, 4);
    }
    #[test]
    fn master_volume_balance_mono_and_clipping() {
        let o = Output {
            volume: 100,
            muted: false,
            mono: false,
            balance: 0,
        };
        assert_eq!(master_gain(o), 65536);
        assert_eq!(master_gain(Output { volume: 50, ..o }), 16384);
        assert_eq!(master_gain(Output { muted: true, ..o }), 0);
        let mut out = [0u8; 8];
        finish(&[[40_000, -40_000], [100, 300]], o, &mut out);
        assert_eq!(i16::from_le_bytes([out[0], out[1]]), i16::MAX);
        assert_eq!(i16::from_le_bytes([out[2], out[3]]), i16::MIN);
        finish(&[[100, 300], [0, 0]], Output { mono: true, ..o }, &mut out);
        assert_eq!(i16::from_le_bytes([out[0], out[1]]), 200);
        assert_eq!(i16::from_le_bytes([out[2], out[3]]), 200);
        finish(
            &[[1000, 1000], [0, 0]],
            Output { balance: 50, ..o },
            &mut out,
        );
        assert_eq!(i16::from_le_bytes([out[0], out[1]]), 500, "left halved");
        assert_eq!(i16::from_le_bytes([out[2], out[3]]), 1000);
    }
    #[test]
    fn the_test_tone_is_a_440_hz_sine_for_one_second() {
        let mut t = Tone::new(440.0, 0.3, 48_000);
        let samples: Vec<i32> = (0..48_000).map(|_| t.next()).collect();
        assert_eq!(t.left, 0);
        let peak = samples.iter().map(|s| s.abs()).max().unwrap();
        assert!((9_700..=9_900).contains(&peak), "amplitude 0.3: {peak}");
        let crossings = samples.windows(2).filter(|w| w[0] < 0 && w[1] >= 0).count();
        assert!((439..=441).contains(&crossings), "{crossings} cycles");
    }
}
