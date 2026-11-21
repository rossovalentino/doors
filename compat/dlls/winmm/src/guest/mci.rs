//! The Media Control Interface for Doors' two MCI devices, WaveAudio and
//! Sequencer: command strings (mciSendString) and the core commands of
//! mciSendCommand. A wave file plays on one of winmm's own wave devices
//! (as PlaySound's do); a MIDI file plays through the synthesizer, which
//! the sequencer holds as an application would (MCIERR_SEQ_PORT_INUSE
//! while the application has it open). The WaveAudio device records PCM
//! from the microphone (a door.audio capture ring, converted by waveIn's
//! recorder) into its element, inserting at the position or overwriting,
//! and saves it as a RIFF WAVE file.
use super::*;
use crate::mci::{self as parse, TimeFormat, err};
use crate::smf::Song;

const MM_MCINOTIFY: u32 = 0x3B9;
const NOTIFY_SUCCESSFUL: u64 = 1;
const NOTIFY_SUPERSEDED: u64 = 2;
const NOTIFY_ABORTED: u64 = 4;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Stopped,
    Playing,
    Paused,
    Recording,
}
/// A recording in progress: the capture ring (section, mapping, mapped
/// bytes) and stream, the converter, what it captured so far, where it goes
/// and where it stops.
struct Take {
    section: u64,
    ring: u64,
    mapped: u64,
    capture: u32,
    recorder: crate::Recorder,
    data: Vec<u8>,
    from: u64,
    to: Option<u64>,
    overwrite: bool,
}
enum Media {
    /// A wave file: its format and its samples as door.audio takes them.
    Wave { wave: Wave, pcm: Arc<Vec<u8>> },
    /// A MIDI file and the synthesizer handle while it plays.
    Song { song: Song, handle: Option<u64> },
    /// A device opened without a file: answers what it is, nothing else.
    Empty,
}
struct Device {
    id: u32,
    alias: String,
    sequencer: bool,
    file: String,
    media: Media,
    format: Format,
    mode: Mode,
    /// Where playback stands (frames for a wave file, microseconds for a
    /// song) when not playing, and where the current play began and ends.
    position: u64,
    started: u64,
    end: u64,
    /// The wave device playing (winmm's DEVICES handle).
    playing: Option<u64>,
    /// The window MM_MCINOTIFY goes to.
    notify: Option<u64>,
    /// Opened by a command naming a file directly: closed when it ends.
    auto: bool,
    take: Option<Take>,
}
/// The time formats each device keeps.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Format {
    Time(TimeFormat),
    SongPointer,
}
impl Format {
    fn name(self) -> &'static str {
        match self {
            Self::Time(t) => t.name(),
            Self::SongPointer => "song pointer",
        }
    }
}
static DEVICES_MCI: Lock<Vec<Device>> = Lock::new(Vec::new());
static NEXT_ID: AtomicU32 = AtomicU32::new(1);

// ---- positions ------------------------------------------------------------------
impl Device {
    /// The length in the device's internal unit.
    fn length(&self) -> u64 {
        match &self.media {
            Media::Wave { wave, pcm } => (pcm.len() / wave.stream_frame()) as u64,
            Media::Song { song, .. } => song.micros(song.end),
            Media::Empty => 0,
        }
    }
    /// An internal position in the time format.
    fn show(&self, value: u64) -> u64 {
        match (&self.media, self.format) {
            (Media::Wave { wave, .. }, Format::Time(TimeFormat::Milliseconds)) => {
                value * 1000 / u64::from(wave.rate)
            }
            (Media::Wave { .. }, Format::Time(TimeFormat::Samples)) => value,
            (Media::Wave { wave, .. }, Format::Time(TimeFormat::Bytes)) => {
                value * u64::from(wave.block_align)
            }
            (Media::Song { song, .. }, Format::SongPointer) => {
                song.tick_at(value) * 4 / u64::from(song.division)
            }
            (Media::Song { .. }, _) => value / 1000,
            _ => 0,
        }
    }
    /// A position given in the time format, as an internal position.
    fn take(&self, value: u64) -> u64 {
        match (&self.media, self.format) {
            (Media::Wave { wave, .. }, Format::Time(TimeFormat::Milliseconds)) => {
                value * u64::from(wave.rate) / 1000
            }
            (Media::Wave { .. }, Format::Time(TimeFormat::Samples)) => value,
            (Media::Wave { wave, .. }, Format::Time(TimeFormat::Bytes)) => {
                value / u64::from(wave.block_align)
            }
            (Media::Song { song, .. }, Format::SongPointer) => {
                song.micros(value * u64::from(song.division) / 4)
            }
            (Media::Song { .. }, _) => value * 1000,
            _ => 0,
        }
    }
    /// Where playback is now (internal unit).
    fn now(&self) -> u64 {
        if self.mode == Mode::Stopped {
            return self.position;
        }
        if let (Some(t), Media::Wave { wave, .. }) = (&self.take, &self.media) {
            return t.from + (t.data.len() / usize::from(wave.block_align)) as u64;
        }
        match &self.media {
            Media::Wave { .. } => {
                let played = self
                    .playing
                    .and_then(|h| {
                        DEVICES.with(|d| d.iter().find(|x| x.handle == h).map(|x| x.stream))
                    })
                    .and_then(played)
                    .map_or(self.end - self.started, |(f, _)| f);
                (self.started + played).min(self.end)
            }
            Media::Song { handle, .. } => {
                let frames = handle.and_then(midi::sequencer_state).map_or(0, |(f, _)| f);
                (self.started + frames * 1_000_000 / u64::from(crate::synth::RATE)).min(self.end)
            }
            Media::Empty => 0,
        }
    }
}

// ---- playback -------------------------------------------------------------------
fn notify(window: u64, code: u64, id: u32) {
    let post = user32(c"PostMessageW");
    if post != 0 && window != 0 {
        let post: PostMessage =
            unsafe { core::mem::transmute::<usize, PostMessage>(post as usize) };
        post(window as usize, MM_MCINOTIFY, code as usize, id as isize);
    }
}
/// Stops whatever `d` plays (keeping its position); answers the window a
/// pending notification goes to.
fn halt(d: &mut Device) {
    if d.mode == Mode::Stopped {
        return;
    }
    if d.mode == Mode::Recording {
        commit(d);
        return;
    }
    d.position = d.now();
    if let Some(handle) = d.playing.take() {
        close_device(handle);
    }
    if let Media::Song { handle, .. } = &mut d.media
        && let Some(h) = handle.take()
    {
        midi::sequencer_close(h);
    }
    d.mode = Mode::Stopped;
}
fn play(d: &mut Device, from: Option<u64>, to: Option<u64>) -> Result<(), u32> {
    let length = d.length();
    let from = from.unwrap_or(if d.position >= length { 0 } else { d.position });
    let to = to.unwrap_or(length);
    if from > length || to > length || from > to {
        return Err(err::OUTOFRANGE);
    }
    halt(d);
    d.started = from;
    d.end = to;
    match &mut d.media {
        Media::Wave { wave, pcm } => {
            let frame = wave.stream_frame();
            let bytes = Arc::new(pcm[from as usize * frame..to as usize * frame].to_vec());
            let stream = open_stream(wave).map_err(|_| err::WAVE_OUTPUTSINUSE)?;
            let handle = NEXT_HANDLE.fetch_add(0x10, Ordering::Relaxed);
            let length = bytes.len();
            let wave = *wave;
            DEVICES.with(|devices| {
                let mut device = super::Device {
                    handle,
                    wave,
                    stream,
                    callback: (CALLBACK_NULL, 0, 0),
                    pending: VecDeque::from([Buffer::new(
                        Data::Sound,
                        Source::Owned(bytes),
                        length,
                        1,
                    )]),
                    playing: VecDeque::new(),
                    sections: VecDeque::new(),
                    submitted: 0,
                    paused: false,
                    volume: 0xFFFF_FFFF,
                    sound: Some(SoundMode::Once),
                    lost: false,
                };
                feed(&mut device);
                devices.push(device);
            });
            d.playing = Some(handle);
            start_worker();
        }
        Media::Song { song, handle } => {
            let h = midi::sequencer_open().map_err(|_| err::SEQ_PORT_INUSE)?;
            let start = song.tick_at(from);
            let end = if to >= length {
                song.end + 1
            } else {
                song.tick_at(to)
            };
            let mut events = song.from(start, end);
            if to < length {
                // Cut short: every channel's notes end at `to`.
                let played: u64 = events.iter().map(|(delta, _)| u64::from(*delta)).sum();
                let rest = (end - start).saturating_sub(played) as u32;
                for channel in 0..16u32 {
                    let delta = if channel == 0 { rest } else { 0 };
                    events.push((delta, crate::smf::Event::Short(0x7BB0 | channel)));
                }
            }
            midi::sequencer_play(h, song.division, &events)?;
            *handle = Some(h);
        }
        Media::Empty => return Err(err::UNSUPPORTED_FUNCTION),
    }
    d.mode = Mode::Playing;
    Ok(())
}
// ---- recording ------------------------------------------------------------------
/// Starts recording into `d`'s wave element at `from` (its position when
/// none), until `to` when given.
fn record(d: &mut Device, from: Option<u64>, to: Option<u64>, overwrite: bool) -> Result<(), u32> {
    let Media::Wave { wave, .. } = &d.media else {
        return Err(err::UNSUPPORTED_FUNCTION);
    };
    if wave.encoding != crate::Encoding::Pcm {
        return Err(err::UNSUPPORTED_FUNCTION);
    }
    let wave = *wave;
    halt(d);
    let length = d.length();
    let from = from.unwrap_or(d.position).min(length);
    if to.is_some_and(|t| t <= from) {
        return Err(err::OUTOFRANGE);
    }
    let (section, ring, mapped) = create_ring().ok_or(err::OUT_OF_MEMORY)?;
    // Refused by Privacy & security > Microphone, or no input.
    let capture = match audio(|a| a.capture_open(section, &executable_name())) {
        Some(Ok(id)) => id,
        Some(Err(door_service_client::Error::Service(door_service_protocol::Code::Denied))) => {
            release_ring(section, ring, mapped);
            return Err(err::WAVE_INPUTSUNSUITABLE);
        }
        _ => {
            release_ring(section, ring, mapped);
            return Err(err::WAVE_INPUTSINUSE);
        }
    };
    d.take = Some(Take {
        section,
        ring,
        mapped,
        capture,
        recorder: crate::Recorder::new(wave),
        data: Vec::new(),
        from,
        to,
        overwrite,
    });
    d.mode = Mode::Recording;
    start_worker();
    Ok(())
}
/// Moves what the ring holds into the take; whether it reached its end.
fn take_in(d: &mut Device) -> bool {
    let Media::Wave { wave, .. } = &d.media else {
        return true;
    };
    let block = usize::from(wave.block_align);
    let Some(t) = d.take.as_mut() else {
        return true;
    };
    let written = ring_count(t.ring, ring::WRITE).load(Ordering::Acquire);
    let mut read = ring_count(t.ring, ring::READ).load(Ordering::Relaxed);
    let data = t.ring + ring::HEADER as u64;
    while read < written {
        let at = (read % RING_BYTES as u64) as usize;
        let n = ((written - read) as usize)
            .min(RING_BYTES - at)
            .min(ring::PERIOD_BYTES * 10);
        // SAFETY: the producer finished these bytes (Acquire above).
        let bytes = unsafe { core::slice::from_raw_parts((data + at as u64) as *const u8, n) };
        t.recorder.push(bytes, &mut t.data);
        read += n as u64;
    }
    ring_count(t.ring, ring::READ).store(read, Ordering::Release);
    match t.to {
        Some(to) => {
            let wanted = (to - t.from) as usize * block;
            if t.data.len() >= wanted {
                t.data.truncate(wanted);
                return true;
            }
            false
        }
        None => ring_word(t.ring, ring::STATE).load(Ordering::Acquire) != ring::RUNNING,
    }
}
/// Ends the recording: its samples go into the element (inserted at its
/// start, or over what was there), and the position follows them.
fn commit(d: &mut Device) {
    let _ = take_in(d);
    let Some(t) = d.take.take() else {
        return;
    };
    let _ = audio(|a| a.capture_close(t.capture));
    release_ring(t.section, t.ring, t.mapped);
    if let Media::Wave { wave, pcm } = &mut d.media {
        let block = usize::from(wave.block_align);
        let samples = Arc::make_mut(pcm);
        let at = (t.from as usize * block).min(samples.len());
        let end = if t.overwrite {
            (at + t.data.len()).min(samples.len())
        } else {
            at
        };
        samples.splice(at..end, t.data.iter().copied());
        d.position = t.from + (t.data.len() / block) as u64;
    }
    d.mode = Mode::Stopped;
}
/// A RIFF WAVE image of `wave` holding `pcm`.
fn wave_file(wave: &Wave, pcm: &[u8]) -> Vec<u8> {
    let mut f = Vec::with_capacity(44 + pcm.len());
    let u32le = |v: u32| v.to_le_bytes();
    f.extend_from_slice(b"RIFF");
    f.extend_from_slice(&u32le(36 + pcm.len() as u32));
    f.extend_from_slice(b"WAVEfmt ");
    f.extend_from_slice(&u32le(16));
    f.extend_from_slice(&1u16.to_le_bytes());
    f.extend_from_slice(&wave.channels.to_le_bytes());
    f.extend_from_slice(&u32le(wave.rate));
    f.extend_from_slice(&u32le(wave.rate * u32::from(wave.block_align)));
    f.extend_from_slice(&wave.block_align.to_le_bytes());
    f.extend_from_slice(&wave.bits.to_le_bytes());
    f.extend_from_slice(b"data");
    f.extend_from_slice(&u32le(pcm.len() as u32));
    f.extend_from_slice(pcm);
    f
}
fn save(d: &mut Device, path: Option<&str>) -> Result<(), u32> {
    if d.mode == Mode::Recording {
        commit(d);
    }
    let Media::Wave { wave, pcm } = &d.media else {
        return Err(err::UNSUPPORTED_FUNCTION);
    };
    let path = match path {
        Some(p) => String::from(p),
        None if !d.file.is_empty() && !d.file.eq_ignore_ascii_case("new") => d.file.clone(),
        None => return Err(err::FILENAME_REQUIRED),
    };
    if !write_file(&path, &wave_file(wave, pcm)) {
        return Err(err::FILE_NOT_SAVED);
    }
    d.file = path;
    Ok(())
}
/// `set` of the element's format (bitspersample, samplespersec, channels,
/// format tag pcm): only while it holds nothing.
fn set_format(d: &mut Device, args: &[String]) -> Result<bool, u32> {
    let Media::Wave { wave, pcm } = &mut d.media else {
        return Ok(false);
    };
    let (mut rate, mut channels, mut bits) = (wave.rate, wave.channels, wave.bits);
    let mut any = false;
    let mut i = 0;
    while i < args.len() {
        let next = args.get(i + 1).and_then(|v| v.parse::<u32>().ok());
        match args[i].as_str() {
            "bitspersample" => bits = next.ok_or(err::BAD_INTEGER)? as u16,
            "samplespersec" => rate = next.ok_or(err::BAD_INTEGER)?,
            "channels" => channels = next.ok_or(err::BAD_INTEGER)? as u16,
            "format" if args.get(i + 1).is_some_and(|w| w == "tag") => {
                if args.get(i + 2).is_none_or(|w| w != "pcm" && w != "1") {
                    return Err(err::UNSUPPORTED_FUNCTION);
                }
                any = true;
                i += 3;
                continue;
            }
            "alignment" | "bytespersec" => {}
            _ => {
                i += 1;
                continue;
            }
        }
        any = true;
        i += 2;
    }
    if !any {
        return Ok(false);
    }
    if !pcm.is_empty() {
        return Err(err::NONAPPLICABLE_FUNCTION);
    }
    let block = channels * bits / 8;
    let mut f = [0u8; 16];
    f[0] = 1;
    f[2..4].copy_from_slice(&channels.to_le_bytes());
    f[4..8].copy_from_slice(&rate.to_le_bytes());
    f[8..12].copy_from_slice(&(rate * u32::from(block)).to_le_bytes());
    f[12..14].copy_from_slice(&block.to_le_bytes());
    f[14..16].copy_from_slice(&bits.to_le_bytes());
    *wave = Wave::parse(&f).ok_or(err::OUTOFRANGE)?;
    Ok(true)
}

/// Whether `d`'s playback has ended.
fn finished(d: &Device) -> bool {
    if d.mode != Mode::Playing {
        return false;
    }
    match &d.media {
        Media::Wave { .. } => d
            .playing
            .is_none_or(|h| !DEVICES.with(|x| x.iter().any(|y| y.handle == h))),
        Media::Song { handle, .. } => handle
            .and_then(midi::sequencer_state)
            .is_none_or(|(_, done)| done),
        Media::Empty => true,
    }
}
/// The worker's pass: ended plays stop at their end and notify.
pub(super) fn pass() -> Vec<Note> {
    let mut notices = Vec::new();
    let mut closing = Vec::new();
    DEVICES_MCI.with(|devices| {
        for d in devices.iter_mut() {
            if d.mode == Mode::Recording && take_in(d) {
                commit(d);
                if let Some(window) = d.notify.take() {
                    notices.push((window, d.id));
                }
                continue;
            }
            if finished(d) {
                halt(d);
                d.position = d.end;
                if let Some(window) = d.notify.take() {
                    notices.push((window, d.id));
                }
                if d.auto {
                    closing.push(d.id);
                }
            }
        }
        devices.retain(|d| !closing.contains(&d.id));
    });
    for (window, id) in notices {
        notify(window, NOTIFY_SUCCESSFUL, id);
    }
    Vec::new()
}
fn wait_for(id: u32) {
    loop {
        let done = DEVICES_MCI.with(|devices| {
            devices.iter_mut().find(|d| d.id == id).is_none_or(|d| {
                if d.mode == Mode::Recording {
                    if take_in(d) {
                        commit(d);
                        return true;
                    }
                    return false;
                }
                if finished(d) {
                    halt(d);
                    d.position = d.end;
                    true
                } else {
                    d.mode != Mode::Playing
                }
            })
        });
        if done {
            return;
        }
        unsafe { DoorSleep(10) };
    }
}

// ---- opening --------------------------------------------------------------------
fn load(path: &str, sequencer: bool) -> Result<Media, u32> {
    let image = read_file(path).ok_or(err::FILE_NOT_FOUND)?;
    if sequencer {
        let song = crate::smf::parse(&image).ok_or(err::INVALID_FILE)?;
        Ok(Media::Song { song, handle: None })
    } else {
        let (wave, range) = crate::parse_wav(&image).ok_or(err::INVALID_FILE)?;
        Ok(Media::Wave {
            wave,
            pcm: Arc::new(wave.convert(&image[range])),
        })
    }
}
/// Opens a device: `kind` ("waveaudio"/"sequencer") or from the file's
/// extension; answers its id.
fn open(
    element: Option<&str>,
    kind: Option<&str>,
    alias: Option<&str>,
    auto: bool,
) -> Result<u32, u32> {
    let kind = match kind {
        Some(k) => k.to_ascii_lowercase(),
        None => match element {
            Some(e)
                if e.eq_ignore_ascii_case("waveaudio") || e.eq_ignore_ascii_case("sequencer") =>
            {
                e.to_ascii_lowercase()
            }
            Some(e) => String::from(parse::type_for_extension(e).ok_or(err::EXTENSION_NOT_FOUND)?),
            None => return Err(err::MISSING_DEVICE_NAME),
        },
    };
    let sequencer = match kind.as_str() {
        "waveaudio" => false,
        "sequencer" => true,
        _ => return Err(err::INVALID_DEVICE_NAME),
    };
    if !device_present() {
        return Err(err::CANNOT_LOAD_DRIVER);
    }
    // "open waveaudio" names the device; "open x.wav type waveaudio" a file.
    let file = element.filter(|e| !e.eq_ignore_ascii_case(&kind));
    let media = match file {
        Some(f) if f.eq_ignore_ascii_case("new") => {
            if alias.is_none() {
                return Err(err::NEW_REQUIRES_ALIAS);
            }
            if sequencer {
                return Err(err::UNSUPPORTED_FUNCTION);
            }
            // A new recording: 11.025 kHz 8-bit mono, empty.
            let wave = Wave::parse(&[1, 0, 1, 0, 0x11, 0x2B, 0, 0, 0x11, 0x2B, 0, 0, 1, 0, 8, 0])
                .ok_or(err::INTERNAL)?;
            Media::Wave {
                wave,
                pcm: Arc::new(Vec::new()),
            }
        }
        Some(f) => load(f, sequencer)?,
        None => Media::Empty,
    };
    let name = String::from(alias.or(file).unwrap_or(&kind)).to_ascii_lowercase();
    let id = DEVICES_MCI.with(|devices| {
        if devices.iter().any(|d| d.alias == name) {
            return Err(if alias.is_some() {
                err::DUPLICATE_ALIAS
            } else {
                err::DEVICE_OPEN
            });
        }
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        devices.push(Device {
            id,
            alias: name.clone(),
            sequencer,
            file: String::from(file.unwrap_or("")),
            media,
            format: if sequencer {
                Format::SongPointer
            } else {
                Format::Time(TimeFormat::Milliseconds)
            },
            mode: Mode::Stopped,
            position: 0,
            started: 0,
            end: 0,
            playing: None,
            notify: None,
            auto,
            take: None,
        });
        Ok(id)
    })?;
    start_worker();
    Ok(id)
}
fn close(id: u32) {
    let device = DEVICES_MCI.with(|devices| {
        devices
            .iter()
            .position(|d| d.id == id)
            .map(|i| devices.remove(i))
    });
    if let Some(mut d) = device {
        halt(&mut d);
        if let Some(window) = d.notify {
            notify(window, NOTIFY_ABORTED, d.id);
        }
    }
}
fn find(name: &str) -> Option<u32> {
    let name = name.to_ascii_lowercase();
    DEVICES_MCI.with(|devices| devices.iter().find(|d| d.alias == name).map(|d| d.id))
}
fn with<R>(id: u32, f: impl FnOnce(&mut Device) -> Result<R, u32>) -> Result<R, u32> {
    DEVICES_MCI.with(|devices| match devices.iter_mut().find(|d| d.id == id) {
        Some(d) => f(d),
        None => Err(err::INVALID_DEVICE_ID),
    })
}

// ---- command strings ------------------------------------------------------------
fn number(word: Option<&String>) -> Result<u64, u32> {
    let w = word.ok_or(err::MISSING_PARAMETER)?;
    w.parse::<u64>().map_err(|_| err::BAD_INTEGER)
}
/// Runs one command line; the reply text.
fn command(line: &str, window: u64) -> Result<String, u32> {
    let words = parse::words(line)?;
    let Some(verb) = words.first().map(|w| w.to_ascii_lowercase()) else {
        return Err(err::MISSING_COMMAND_STRING);
    };
    let Some(name) = words.get(1) else {
        return Err(err::MISSING_DEVICE_NAME);
    };
    let args: Vec<String> = words[2..].iter().map(|w| w.to_ascii_lowercase()).collect();
    let flag = |f: &str| args.iter().any(|a| a == f);
    let value = |f: &str| {
        args.iter()
            .position(|a| a == f)
            .and_then(|i| words.get(2 + i + 1))
    };
    let wait = flag("wait");
    let notify_window = flag("notify").then_some(window);
    match verb.as_str() {
        "open" => {
            let id = open(
                Some(name),
                value("type").map(String::as_str),
                value("alias").map(String::as_str),
                false,
            )?;
            if let Some(w) = notify_window {
                notify(w, NOTIFY_SUCCESSFUL, id);
            }
            return Ok(alloc::format!("{id}"));
        }
        "sysinfo" => {
            let all = name.eq_ignore_ascii_case("all");
            let names: Vec<&str> = parse::DEVICES
                .iter()
                .copied()
                .filter(|d| all || d.eq_ignore_ascii_case(name))
                .collect();
            if !all && names.is_empty() {
                return Err(err::INVALID_DEVICE_NAME);
            }
            if flag("quantity") {
                let n = if flag("open") {
                    DEVICES_MCI.with(|d| {
                        d.iter()
                            .filter(|x| {
                                all || names.iter().any(|n| {
                                    n.eq_ignore_ascii_case(if x.sequencer {
                                        "sequencer"
                                    } else {
                                        "waveaudio"
                                    })
                                })
                            })
                            .count()
                    })
                } else {
                    names.len()
                };
                return Ok(alloc::format!("{n}"));
            }
            if flag("name") {
                let n = number(value("name"))? as usize;
                return names
                    .get(n.wrapping_sub(1))
                    .map(|s| String::from(*s))
                    .ok_or(err::OUTOFRANGE);
            }
            return Err(err::MISSING_PARAMETER);
        }
        "close" if name.eq_ignore_ascii_case("all") => {
            let ids: Vec<u32> = DEVICES_MCI.with(|d| d.iter().map(|x| x.id).collect());
            ids.into_iter().for_each(close);
            return Ok(String::new());
        }
        "capability" if parse::DEVICES.iter().any(|d| d.eq_ignore_ascii_case(name)) => {
            return capability(name.eq_ignore_ascii_case("sequencer"), &args);
        }
        _ => {}
    }
    // Every other command names an open device, or a file to open for it.
    let id = match find(name) {
        Some(id) => id,
        None if verb == "play" && parse::type_for_extension(name).is_some() => {
            open(Some(name), None, None, true)?
        }
        None => return Err(err::INVALID_DEVICE_NAME),
    };
    let reply = match verb.as_str() {
        "close" => {
            close(id);
            String::new()
        }
        "play" => {
            let superseded = with(id, |d| {
                let from = value("from")
                    .map(|_| number(value("from")))
                    .transpose()?
                    .map(|v| d.take(v));
                let to = value("to")
                    .map(|_| number(value("to")))
                    .transpose()?
                    .map(|v| d.take(v));
                let previous = d.notify.take();
                play(d, from, to)?;
                d.notify = notify_window;
                Ok(previous)
            })?;
            if let Some(w) = superseded {
                notify(w, NOTIFY_SUPERSEDED, id);
            }
            if wait {
                wait_for(id);
                if notify_window.is_some() {
                    with(id, |d| Ok(d.notify.take()))?;
                    notify(window, NOTIFY_SUCCESSFUL, id);
                }
            }
            String::new()
        }
        "record" => {
            let superseded = with(id, |d| {
                let from = value("from")
                    .map(|_| number(value("from")))
                    .transpose()?
                    .map(|v| d.take(v));
                let to = value("to")
                    .map(|_| number(value("to")))
                    .transpose()?
                    .map(|v| d.take(v));
                let previous = d.notify.take();
                record(d, from, to, flag("overwrite"))?;
                d.notify = notify_window;
                Ok(previous)
            })?;
            if let Some(w) = superseded {
                notify(w, NOTIFY_SUPERSEDED, id);
            }
            if wait {
                wait_for(id);
                if notify_window.is_some() {
                    with(id, |d| Ok(d.notify.take()))?;
                    notify(window, NOTIFY_SUCCESSFUL, id);
                }
            }
            String::new()
        }
        "save" => {
            with(id, |d| {
                save(
                    d,
                    words.get(2).map(String::as_str).filter(|w| {
                        !w.eq_ignore_ascii_case("wait") && !w.eq_ignore_ascii_case("notify")
                    }),
                )
            })?;
            if let Some(w) = notify_window {
                notify(w, NOTIFY_SUCCESSFUL, id);
            }
            String::new()
        }
        "delete" => {
            with(id, |d| {
                if d.mode != Mode::Stopped {
                    halt(d);
                }
                let length = d.length();
                let from = value("from")
                    .map(|_| number(value("from")))
                    .transpose()?
                    .map_or(0, |v| d.take(v));
                let to = value("to")
                    .map(|_| number(value("to")))
                    .transpose()?
                    .map_or(length, |v| d.take(v));
                if from > to || to > length {
                    return Err(err::OUTOFRANGE);
                }
                let Media::Wave { wave, pcm } = &mut d.media else {
                    return Err(err::UNSUPPORTED_FUNCTION);
                };
                let block = usize::from(wave.block_align);
                Arc::make_mut(pcm).drain(from as usize * block..to as usize * block);
                d.position = from;
                Ok(())
            })?;
            String::new()
        }
        "cue" => {
            with(id, |_| Ok(()))?;
            String::new()
        }
        "stop" | "pause" => {
            let aborted = with(id, |d| {
                if matches!(d.media, Media::Empty) {
                    return Err(err::UNSUPPORTED_FUNCTION);
                }
                if verb == "pause" && d.mode == Mode::Playing {
                    pause(d, true);
                    Ok(None)
                } else {
                    halt(d);
                    Ok(d.notify.take())
                }
            })?;
            if let Some(w) = aborted {
                notify(w, NOTIFY_ABORTED, id);
            }
            String::new()
        }
        "resume" => {
            with(id, |d| {
                if d.mode == Mode::Paused {
                    pause(d, false);
                }
                Ok(())
            })?;
            String::new()
        }
        "seek" => {
            with(id, |d| {
                if matches!(d.media, Media::Empty) {
                    return Err(err::UNSUPPORTED_FUNCTION);
                }
                let target = match value("to").map(|w| w.to_ascii_lowercase()) {
                    Some(w) if w == "start" => 0,
                    Some(w) if w == "end" => d.length(),
                    Some(_) => d.take(number(value("to"))?),
                    None => return Err(err::MISSING_PARAMETER),
                };
                if target > d.length() {
                    return Err(err::OUTOFRANGE);
                }
                halt(d);
                d.position = target;
                Ok(())
            })?;
            String::new()
        }
        "set" => {
            with(id, |d| {
                if flag("time") && flag("format") {
                    let at = args.iter().position(|a| a == "format").unwrap();
                    let word = args.get(at + 1).ok_or(err::MISSING_PARAMETER)?;
                    let joined = args[at + 1..].join(" ");
                    d.format = match (d.sequencer, TimeFormat::parse(word)) {
                        (false, Some(t)) => Format::Time(t),
                        (true, Some(TimeFormat::Milliseconds)) => {
                            Format::Time(TimeFormat::Milliseconds)
                        }
                        (true, _) if joined == "song pointer" => Format::SongPointer,
                        _ => return Err(err::BAD_TIME_FORMAT),
                    };
                    Ok(())
                } else if flag("audio") || set_format(d, &args)? {
                    Ok(())
                } else {
                    Err(err::MISSING_PARAMETER)
                }
            })?;
            String::new()
        }
        "status" => with(id, |d| status(d, &args))?,
        "capability" => {
            let sequencer = with(id, |d| Ok(d.sequencer))?;
            capability(sequencer, &args)?
        }
        "info" => with(id, |d| {
            if flag("product") {
                Ok(String::from(if d.sequencer {
                    "MIDI Sequencer"
                } else {
                    "Sound"
                }))
            } else if flag("file") {
                Ok(d.file.clone())
            } else {
                Err(err::MISSING_PARAMETER)
            }
        })?,
        _ => return Err(err::UNRECOGNIZED_COMMAND),
    };
    Ok(reply)
}
fn pause(d: &mut Device, paused: bool) {
    match &d.media {
        Media::Wave { .. } => {
            if let Some(stream) = d
                .playing
                .and_then(|h| DEVICES.with(|x| x.iter().find(|y| y.handle == h).map(|y| y.stream)))
            {
                audio(|a| a.pause(stream, paused));
            }
        }
        Media::Song {
            handle: Some(h), ..
        } => midi::sequencer_pause(*h, !paused),
        _ => {}
    }
    d.mode = if paused { Mode::Paused } else { Mode::Playing };
}
fn status(d: &Device, args: &[String]) -> Result<String, u32> {
    let item = args.join(" ");
    let item = item
        .trim_end_matches(" wait")
        .trim_end_matches(" notify")
        .trim();
    // A sequencer opened without a file answers only what it is.
    if matches!(d.media, Media::Empty) && d.sequencer {
        return Err(err::UNSUPPORTED_FUNCTION);
    }
    let yes = |b: bool| String::from(if b { "true" } else { "false" });
    Ok(match item {
        "mode" => String::from(match d.mode {
            Mode::Stopped => "stopped",
            Mode::Playing => "playing",
            Mode::Paused => "paused",
            Mode::Recording => "recording",
        }),
        "length" => alloc::format!("{}", d.show(d.length())),
        "position" => alloc::format!("{}", d.show(d.now())),
        "start position" => String::from("0"),
        "time format" => String::from(d.format.name()),
        "ready" | "media present" => yes(true),
        "current track" | "number of tracks" => String::from("1"),
        _ => match (&d.media, item) {
            (Media::Wave { wave, .. }, "bitspersample") => alloc::format!("{}", wave.bits),
            (Media::Wave { wave, .. }, "samplespersec") => alloc::format!("{}", wave.rate),
            (Media::Wave { wave, .. }, "channels") => alloc::format!("{}", wave.channels),
            (Media::Wave { wave, .. }, "alignment") => alloc::format!("{}", wave.block_align),
            (Media::Wave { wave, .. }, "bytespersec") => {
                alloc::format!("{}", wave.rate * u32::from(wave.block_align))
            }
            (Media::Wave { wave, .. }, "format tag") => String::from(match wave.encoding {
                crate::Encoding::Pcm => "pcm",
                crate::Encoding::Float => "3",
            }),
            (Media::Song { song, .. }, "tempo") => {
                // Beats per minute at the start.
                let tempo = song
                    .events
                    .iter()
                    .find_map(|(_, e)| match e {
                        crate::smf::Event::Tempo(t) => Some(*t),
                        _ => None,
                    })
                    .unwrap_or(500_000);
                alloc::format!("{}", 60_000_000 / tempo.max(1))
            }
            (Media::Song { .. }, "port") => String::from("0"),
            (Media::Song { song, .. }, "division type") => {
                let _ = song;
                String::from("PPQN")
            }
            (Media::Song { .. }, "master" | "slave") => String::from("none"),
            _ => return Err(err::UNRECOGNIZED_KEYWORD),
        },
    })
}
fn capability(sequencer: bool, args: &[String]) -> Result<String, u32> {
    let item = args.join(" ");
    let yes = |b: bool| String::from(if b { "true" } else { "false" });
    Ok(match item.trim() {
        "can play" | "has audio" | "uses files" | "compound device" => yes(true),
        "can record" | "can save" => yes(!sequencer),
        "can eject" | "has video" => yes(false),
        "device type" => String::from(if sequencer { "sequencer" } else { "waveaudio" }),
        "inputs" => String::from(if sequencer { "0" } else { "1" }),
        "outputs" => String::from("1"),
        _ => return Err(err::UNRECOGNIZED_KEYWORD),
    })
}
fn reply(text: &str, out: u64, length: u32, wide: bool) -> u32 {
    if out == 0 || length == 0 {
        return 0;
    }
    let units: Vec<u16> = text.encode_utf16().collect();
    if units.len() >= length as usize {
        return err::PARAM_OVERFLOW;
    }
    let bytes: Vec<u8> = if wide {
        units
            .iter()
            .chain([&0])
            .flat_map(|u| u.to_le_bytes())
            .collect()
    } else {
        text.bytes().chain([0]).collect()
    };
    write(out, &bytes).map_or(err::INTERNAL, |()| 0)
}
fn send_string(command_text: u64, out: u64, length: u32, window: u64, wide: bool) -> u32 {
    let Ok(line) = text_arg(command_text, wide) else {
        return err::NULL_PARAMETER_BLOCK;
    };
    match command(&line, window) {
        Ok(text) => reply(&text, out, length, wide),
        Err(e) => {
            let _ = reply("", out, length, wide);
            e
        }
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn mciSendStringW(
    command: usize,
    out: usize,
    length: u32,
    window: usize,
) -> u32 {
    let command = door_windows_abi::widen(command);
    let out = door_windows_abi::widen(out);
    let window = door_windows_abi::widen(window);
    send_string(command, out, length, window, true)
}
#[unsafe(no_mangle)]
pub extern "system" fn mciSendStringA(
    command: usize,
    out: usize,
    length: u32,
    window: usize,
) -> u32 {
    let command = door_windows_abi::widen(command);
    let out = door_windows_abi::widen(out);
    let window = door_windows_abi::widen(window);
    send_string(command, out, length, window, false)
}
fn error_string(code: u32, out: u64, length: u32, wide: bool) -> i32 {
    let Some(text) = parse::error_text(code) else {
        return 0;
    };
    i32::from(reply(text, out, length, wide) == 0)
}
#[unsafe(no_mangle)]
pub extern "system" fn mciGetErrorStringW(code: u32, out: usize, length: u32) -> i32 {
    let out = door_windows_abi::widen(out);
    error_string(code, out, length, true)
}
#[unsafe(no_mangle)]
pub extern "system" fn mciGetErrorStringA(code: u32, out: usize, length: u32) -> i32 {
    let out = door_windows_abi::widen(out);
    error_string(code, out, length, false)
}
fn device_id(name: u64, wide: bool) -> u32 {
    text_arg(name, wide)
        .ok()
        .and_then(|n| {
            if n.eq_ignore_ascii_case("all") {
                Some(0xFFFF)
            } else {
                find(&n)
            }
        })
        .unwrap_or(0)
}
#[unsafe(no_mangle)]
pub extern "system" fn mciGetDeviceIDW(name: usize) -> u32 {
    let name = door_windows_abi::widen(name);
    device_id(name, true)
}
#[unsafe(no_mangle)]
pub extern "system" fn mciGetDeviceIDA(name: usize) -> u32 {
    let name = door_windows_abi::widen(name);
    device_id(name, false)
}

// ---- mciSendCommand -------------------------------------------------------------
const MCI_OPEN: u32 = 0x803;
const MCI_CLOSE: u32 = 0x804;
const MCI_PLAY: u32 = 0x806;
const MCI_SEEK: u32 = 0x807;
const MCI_STOP: u32 = 0x808;
const MCI_PAUSE: u32 = 0x809;
const MCI_SET: u32 = 0x80D;
const MCI_STATUS: u32 = 0x814;
const MCI_RESUME: u32 = 0x855;
const MCI_RECORD: u32 = 0x80F;
const MCI_SAVE: u32 = 0x813;
const MCI_RECORD_OVERWRITE: u32 = 0x200;
const MCI_NOTIFY: u32 = 0x1;
const MCI_WAIT: u32 = 0x2;
const MCI_FROM: u32 = 0x4;
const MCI_TO: u32 = 0x8;
const MCI_OPEN_SHAREABLE: u32 = 0x100;
const MCI_OPEN_ELEMENT: u32 = 0x200;
const MCI_OPEN_ALIAS: u32 = 0x400;
const MCI_OPEN_TYPE_ID: u32 = 0x1000;
const MCI_OPEN_TYPE: u32 = 0x2000;
const MCI_SEEK_TO_START: u32 = 0x100;
const MCI_SEEK_TO_END: u32 = 0x200;
const MCI_STATUS_ITEM: u32 = 0x100;
const MCI_SET_TIME_FORMAT: u32 = 0x400;
const MCI_ALL_DEVICE_ID: u32 = 0xFFFF;
/// MCI_DEVTYPE_WAVEFORM_AUDIO and MCI_DEVTYPE_SEQUENCER (MCI_OPEN_TYPE_ID).
const DEVTYPE_WAVE: u64 = 522;
const DEVTYPE_SEQUENCER: u64 = 523;
/// The MCI_*_PARMS structures open with dwCallback (a DWORD_PTR) and
/// mmsystem.h packs them: the fields after it start at P, the program's
/// pointer width (P17).
const P: u64 = super::POINTER as u64;
fn send_command(id: u32, message: u32, flags: u32, params: u64, wide: bool) -> u32 {
    let result = (|| -> Result<(), u32> {
        let callback = if params != 0 && flags & MCI_NOTIFY != 0 {
            super::read_pointer(params)?
        } else {
            0
        };
        match message {
            MCI_OPEN => {
                // MCI_OPEN_PARMS (packed): dwCallback, wDeviceID (P),
                // lpstrDeviceType (P + 4), lpstrElementName, lpstrAlias.
                if params == 0 {
                    return Err(err::NULL_PARAMETER_BLOCK);
                }
                let string = |at: u64| -> Result<Option<String>, u32> {
                    let p = super::read_pointer(params + at)?;
                    if p == 0 {
                        Ok(None)
                    } else {
                        text_arg(p, wide).map(Some)
                    }
                };
                let kind = if flags & MCI_OPEN_TYPE == 0 {
                    None
                } else if flags & MCI_OPEN_TYPE_ID != 0 {
                    Some(String::from(
                        match super::read_pointer(params + P + 4)? & 0xFFFF {
                            DEVTYPE_WAVE => "waveaudio",
                            DEVTYPE_SEQUENCER => "sequencer",
                            _ => return Err(err::INVALID_DEVICE_NAME),
                        },
                    ))
                } else {
                    string(P + 4)?
                };
                let element = if flags & MCI_OPEN_ELEMENT != 0 {
                    string(2 * P + 4)?
                } else {
                    None
                };
                let alias = if flags & MCI_OPEN_ALIAS != 0 {
                    string(3 * P + 4)?
                } else {
                    None
                };
                let _ = MCI_OPEN_SHAREABLE;
                let element = element.or_else(|| kind.clone());
                let new = open(element.as_deref(), kind.as_deref(), alias.as_deref(), false)?;
                write_u32(params + P, new)?;
                if flags & MCI_NOTIFY != 0 {
                    notify(callback, NOTIFY_SUCCESSFUL, new);
                }
                Ok(())
            }
            MCI_CLOSE if id == MCI_ALL_DEVICE_ID => {
                let ids: Vec<u32> = DEVICES_MCI.with(|d| d.iter().map(|x| x.id).collect());
                ids.into_iter().for_each(close);
                Ok(())
            }
            MCI_CLOSE => {
                with(id, |_| Ok(()))?;
                close(id);
                Ok(())
            }
            MCI_PLAY => {
                // MCI_PLAY_PARMS: dwCallback, dwFrom (P), dwTo (P + 4).
                let previous = with(id, |d| {
                    let from = if flags & MCI_FROM != 0 {
                        Some(d.take(u64::from(read_u32(params + P)?)))
                    } else {
                        None
                    };
                    let to = if flags & MCI_TO != 0 {
                        Some(d.take(u64::from(read_u32(params + P + 4)?)))
                    } else {
                        None
                    };
                    let previous = d.notify.take();
                    play(d, from, to)?;
                    d.notify = (flags & MCI_NOTIFY != 0).then_some(callback);
                    Ok(previous)
                })?;
                if let Some(w) = previous {
                    notify(w, NOTIFY_SUPERSEDED, id);
                }
                if flags & MCI_WAIT != 0 {
                    wait_for(id);
                }
                Ok(())
            }
            MCI_STOP | MCI_PAUSE => {
                let aborted = with(id, |d| {
                    if message == MCI_PAUSE && d.mode == Mode::Playing {
                        pause(d, true);
                        Ok(None)
                    } else {
                        halt(d);
                        Ok(d.notify.take())
                    }
                })?;
                if let Some(w) = aborted {
                    notify(w, NOTIFY_ABORTED, id);
                }
                Ok(())
            }
            MCI_RESUME => with(id, |d| {
                if d.mode == Mode::Paused {
                    pause(d, false);
                }
                Ok(())
            }),
            MCI_SEEK => with(id, |d| {
                let target = if flags & MCI_SEEK_TO_START != 0 {
                    0
                } else if flags & MCI_SEEK_TO_END != 0 {
                    d.length()
                } else if flags & MCI_TO != 0 {
                    d.take(u64::from(read_u32(params + P)?))
                } else {
                    return Err(err::MISSING_PARAMETER);
                };
                halt(d);
                d.position = target.min(d.length());
                Ok(())
            }),
            MCI_SET => with(id, |d| {
                // MCI_SET_PARMS: dwCallback, dwTimeFormat (P), dwAudio.
                if flags & MCI_SET_TIME_FORMAT != 0 {
                    d.format = match (d.sequencer, read_u32(params + P)?) {
                        (_, 0) => Format::Time(TimeFormat::Milliseconds),
                        (false, 8) => Format::Time(TimeFormat::Bytes),
                        (false, 9) => Format::Time(TimeFormat::Samples),
                        (true, 0x4001) => Format::SongPointer,
                        _ => return Err(err::BAD_TIME_FORMAT),
                    };
                }
                Ok(())
            }),
            MCI_STATUS => {
                // MCI_STATUS_PARMS: dwCallback, dwReturn (a DWORD_PTR at P),
                // dwItem (2P).
                if flags & MCI_STATUS_ITEM == 0 {
                    return Err(err::MISSING_PARAMETER);
                }
                let item = read_u32(params + 2 * P)?;
                let value = with(id, |d| {
                    Ok(match item {
                        1 => d.show(d.length()),
                        2 => d.show(d.now()),
                        3 | 8 => 1,
                        // MCI_MODE_STOP, _PLAY, _PAUSE.
                        4 => match d.mode {
                            Mode::Stopped => 525,
                            Mode::Playing => 526,
                            Mode::Paused => 529,
                            Mode::Recording => 527,
                        },
                        5 | 7 => 1,
                        6 => match d.format {
                            Format::Time(TimeFormat::Milliseconds) => 0,
                            Format::Time(TimeFormat::Bytes) => 8,
                            Format::Time(TimeFormat::Samples) => 9,
                            Format::SongPointer => 0x4001,
                        },
                        _ => return Err(err::UNRECOGNIZED_KEYWORD),
                    })
                })?;
                super::write_pointer(params + P, value as u64)
            }
            MCI_RECORD => {
                // MCI_RECORD_PARMS: dwCallback, dwFrom (P), dwTo (P + 4).
                let previous = with(id, |d| {
                    let from = if flags & MCI_FROM != 0 {
                        Some(d.take(u64::from(read_u32(params + P)?)))
                    } else {
                        None
                    };
                    let to = if flags & MCI_TO != 0 {
                        Some(d.take(u64::from(read_u32(params + P + 4)?)))
                    } else {
                        None
                    };
                    let previous = d.notify.take();
                    record(d, from, to, flags & MCI_RECORD_OVERWRITE != 0)?;
                    d.notify = (flags & MCI_NOTIFY != 0).then_some(callback);
                    Ok(previous)
                })?;
                if let Some(w) = previous {
                    notify(w, NOTIFY_SUPERSEDED, id);
                }
                if flags & MCI_WAIT != 0 {
                    wait_for(id);
                }
                Ok(())
            }
            MCI_SAVE => {
                // MCI_SAVE_PARMS: dwCallback, lpfilename (P).
                if params == 0 {
                    return Err(err::NULL_PARAMETER_BLOCK);
                }
                let name = super::read_pointer(params + P)?;
                let path = if name == 0 {
                    None
                } else if wide {
                    Some(text_arg(name, true)?)
                } else {
                    Some(text_arg(name, false)?)
                };
                with(id, |d| save(d, path.as_deref()))
            }
            _ => Err(err::UNRECOGNIZED_COMMAND),
        }
    })();
    result.err().unwrap_or(0)
}
#[unsafe(no_mangle)]
pub extern "system" fn mciSendCommandW(id: u32, message: u32, flags: usize, params: usize) -> u32 {
    let flags = door_windows_abi::widen(flags);
    let params = door_windows_abi::widen(params);
    send_command(id, message, flags as u32, params, true)
}
#[unsafe(no_mangle)]
pub extern "system" fn mciSendCommandA(id: u32, message: u32, flags: usize, params: usize) -> u32 {
    let flags = door_windows_abi::widen(flags);
    let params = door_windows_abi::widen(params);
    send_command(id, message, flags as u32, params, false)
}
