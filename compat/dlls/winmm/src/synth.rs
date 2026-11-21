//! The software synthesizer behind winmm's MIDI output device (Windows
//! calls its own "Microsoft GS Wavetable Synth"). Doors has no wavetable
//! sample set (Windows' gm.dls is licensed), so the 128 General MIDI
//! programs and the GM percussion map are synthesized: oscillators chosen
//! per instrument family, an ADSR envelope, a one-pole low-pass filter and
//! synthesized drums. The MIDI handling follows GM/GS: 16 channels (10 is
//! percussion), 32 voices with the oldest stolen, note on/off, sustain
//! pedal, volume, expression, pan, modulation, pitch bend with its RPN
//! range, program and bank select, all-notes/all-sound off, reset all
//! controllers, and the GM, GS and XG reset and master volume system
//! exclusive messages. Output is 48 kHz stereo 16-bit.
use alloc::vec::Vec;

pub const RATE: u32 = 48_000;
pub const VOICES: usize = 32;
pub const CHANNELS: usize = 16;
const DRUMS: usize = 9;
/// Parameters are recomputed every this many frames (1.33 ms).
const BLOCK: usize = 64;
const TABLE: usize = 1024;

/// 2^x (no_std): the nearest integer through the exponent bits, the rest
/// (within half an octave) by its series (relative error below 3e-6).
pub fn exp2(x: f32) -> f32 {
    if x < -126.0 {
        return 0.0;
    }
    if x > 127.0 {
        return f32::MAX;
    }
    let shifted = x + 0.5;
    let truncated = shifted as i32;
    let whole = if (truncated as f32) > shifted {
        truncated - 1
    } else {
        truncated
    };
    let f = x - whole as f32;
    let p = 1.0
        + f * (core::f32::consts::LN_2
            + f * (0.240_226_5 + f * (0.055_504_1 + f * (0.009_618_1 + f * 0.001_333_6))));
    f32::from_bits(((whole + 127) as u32) << 23) * p
}
/// The frequency of MIDI note `note` (fractional: bends), A4 = 440 Hz.
pub fn frequency(note: f32) -> f32 {
    440.0 * exp2((note - 69.0) / 12.0)
}
/// A per-sample multiplier that falls by 60 dB over `ms` milliseconds.
fn decay_factor(ms: f32) -> f32 {
    if ms <= 0.0 {
        return 0.0;
    }
    // 60 dB = 2^-9.966; per sample: 2^(-9.966 / samples).
    exp2(-9.966 / (ms * RATE as f32 / 1000.0))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Shape {
    Sine,
    Triangle,
    Square,
    Saw,
    Pulse,
    Noise,
}
/// An instrument: two oscillators (the second detuned, mixed at `mix`),
/// an envelope (times in ms, sustain 0..1; `hold` false for sounds that
/// decay while the key is down), the filter's brightness (0..1, the
/// cutoff as a fraction of the note's harmonics) and its sweep.
#[derive(Clone, Copy, Debug)]
struct Patch {
    shape: Shape,
    second: Shape,
    detune: f32,
    mix: f32,
    attack: f32,
    decay: f32,
    sustain: f32,
    release: f32,
    bright: f32,
    sweep: f32,
    octave: f32,
}
const fn patch(
    shape: Shape,
    second: Shape,
    detune: f32,
    mix: f32,
    adsr: [f32; 4],
    bright: f32,
    sweep: f32,
) -> Patch {
    Patch {
        shape,
        second,
        detune,
        mix,
        attack: adsr[0],
        decay: adsr[1],
        sustain: adsr[2],
        release: adsr[3],
        bright,
        sweep,
        octave: 0.0,
    }
}
use Shape::*;
/// One patch per GM family (programs 8n..8n+7), varied within the family
/// by `program_patch`.
const FAMILIES: [Patch; 16] = [
    // Piano: bright attack, slow decay while held.
    patch(
        Triangle,
        Saw,
        0.0,
        0.25,
        [2.0, 1800.0, 0.0, 300.0],
        0.6,
        0.5,
    ),
    // Chromatic percussion: bells and mallets.
    patch(Sine, Sine, 1200.0, 0.3, [1.0, 900.0, 0.0, 400.0], 1.0, 0.0),
    // Organ: sustained, no decay.
    patch(Square, Sine, 1200.0, 0.4, [8.0, 0.0, 1.0, 60.0], 0.5, 0.0),
    // Guitar: plucked.
    patch(Saw, Triangle, 5.0, 0.3, [1.0, 1200.0, 0.0, 200.0], 0.8, 0.7),
    // Bass.
    patch(
        Saw,
        Square,
        -1200.0,
        0.3,
        [3.0, 800.0, 0.3, 120.0],
        0.35,
        0.3,
    ),
    // Strings: slow attack, sustained.
    patch(Saw, Saw, 7.0, 0.5, [120.0, 400.0, 0.8, 350.0], 0.45, 0.0),
    // Ensemble.
    patch(Saw, Saw, -7.0, 0.5, [150.0, 300.0, 0.85, 400.0], 0.4, 0.0),
    // Brass.
    patch(Saw, Saw, 4.0, 0.4, [40.0, 300.0, 0.75, 150.0], 0.55, 0.2),
    // Reed.
    patch(Square, Saw, 0.0, 0.2, [25.0, 200.0, 0.85, 100.0], 0.45, 0.0),
    // Pipe: flutes, mostly sine.
    patch(
        Sine,
        Triangle,
        0.0,
        0.25,
        [40.0, 200.0, 0.9, 120.0],
        0.8,
        0.0,
    ),
    // Synth lead.
    patch(Square, Saw, 6.0, 0.4, [5.0, 200.0, 0.85, 80.0], 0.8, 0.0),
    // Synth pad.
    patch(
        Saw,
        Triangle,
        9.0,
        0.5,
        [300.0, 800.0, 0.7, 700.0],
        0.35,
        0.0,
    ),
    // Synth effects.
    patch(Pulse, Sine, 12.0, 0.5, [80.0, 1500.0, 0.5, 800.0], 0.5, 0.3),
    // Ethnic: plucked strings.
    patch(Triangle, Saw, 0.0, 0.3, [1.0, 1000.0, 0.0, 250.0], 0.7, 0.6),
    // Percussive.
    patch(Sine, Noise, 0.0, 0.1, [1.0, 400.0, 0.0, 200.0], 0.7, 0.5),
    // Sound effects.
    patch(Noise, Saw, 0.0, 0.3, [20.0, 800.0, 0.5, 400.0], 0.4, 0.0),
];
fn program_patch(program: u8) -> Patch {
    let mut p = FAMILIES[usize::from(program / 8) % 16];
    let index = program % 8;
    match program {
        // Electric pianos, harpsichord, clavi: brighter.
        4..=7 => p.bright = 0.85,
        // Celesta, glockenspiel, music box: an octave up.
        8..=10 => p.octave = 1.0,
        // Rock and church organs: brighter.
        18 | 19 => p.bright = 0.8,
        // Clean electric guitars sustain a little.
        26..=28 => p.sustain = 0.2,
        // Overdriven and distortion guitars: square, sustained.
        29 | 30 => {
            p.shape = Square;
            p.sustain = 0.6;
            p.bright = 0.9;
        }
        // Acoustic and fingered basses decay; synth basses are brighter.
        38 | 39 => p.bright = 0.7,
        // Pizzicato strings and harp: plucked.
        45 | 46 => {
            p.attack = 1.0;
            p.decay = 600.0;
            p.sustain = 0.0;
        }
        // Timpani.
        47 => {
            p.shape = Sine;
            p.decay = 900.0;
            p.sustain = 0.0;
        }
        // Choir and voices: softer.
        52..=54 => p.bright = 0.3,
        // Square and saw leads.
        80 => {
            p.shape = Square;
            p.mix = 0.0;
        }
        81 => {
            p.shape = Saw;
            p.mix = 0.0;
        }
        _ => {
            // Within a family, step the brightness a little per program so
            // neighbours differ.
            p.bright = (p.bright + f32::from(index) * 0.03).min(1.0);
        }
    }
    p
}

/// A percussion sound (the GM key map, channel 10).
#[derive(Clone, Copy, Debug, PartialEq)]
enum Drum {
    /// A sine sweeping from `from` to `to` Hz, decaying over `ms`.
    Tone { from: f32, to: f32, ms: f32 },
    /// Noise through a filter (`high`: high-passed), decaying over `ms`,
    /// with an optional tone at `tone` Hz.
    Noise { high: bool, ms: f32, tone: f32 },
}
fn drum(key: u8) -> Option<Drum> {
    Some(match key {
        35 | 36 => Drum::Tone {
            from: 150.0,
            to: 48.0,
            ms: 350.0,
        },
        37 => Drum::Noise {
            high: true,
            ms: 40.0,
            tone: 1700.0,
        },
        38 | 40 => Drum::Noise {
            high: false,
            ms: 180.0,
            tone: 185.0,
        },
        39 => Drum::Noise {
            high: true,
            ms: 120.0,
            tone: 0.0,
        },
        // Low floor tom .. high tom.
        41 | 43 | 45 | 47 | 48 | 50 => {
            let f = 70.0 + f32::from(key - 41) * 18.0;
            Drum::Tone {
                from: f * 1.5,
                to: f,
                ms: 400.0,
            }
        }
        42 | 44 => Drum::Noise {
            high: true,
            ms: 50.0,
            tone: 0.0,
        },
        46 => Drum::Noise {
            high: true,
            ms: 350.0,
            tone: 0.0,
        },
        49 | 52 | 55 | 57 => Drum::Noise {
            high: true,
            ms: 1400.0,
            tone: 0.0,
        },
        51 | 53 | 59 => Drum::Noise {
            high: true,
            ms: 900.0,
            tone: 520.0,
        },
        54 => Drum::Noise {
            high: true,
            ms: 120.0,
            tone: 0.0,
        },
        56 => Drum::Tone {
            from: 560.0,
            to: 540.0,
            ms: 250.0,
        },
        // Bongos, congas, timbales.
        60..=66 => {
            let f = 180.0 + f32::from(key - 60) * 45.0;
            Drum::Tone {
                from: f * 1.2,
                to: f,
                ms: 200.0,
            }
        }
        67 | 68 => Drum::Tone {
            from: 900.0,
            to: 880.0,
            ms: 150.0,
        },
        69 | 70 | 82 => Drum::Noise {
            high: true,
            ms: 80.0,
            tone: 0.0,
        },
        71 | 72 => Drum::Tone {
            from: 2600.0,
            to: 2400.0,
            ms: 120.0,
        },
        73 | 74 => Drum::Noise {
            high: true,
            ms: 160.0,
            tone: 0.0,
        },
        75..=77 => Drum::Tone {
            from: 1800.0,
            to: 1700.0,
            ms: 90.0,
        },
        78 | 79 => Drum::Tone {
            from: 400.0,
            to: 300.0,
            ms: 180.0,
        },
        80 | 81 => Drum::Tone {
            from: 3500.0,
            to: 3500.0,
            ms: 600.0,
        },
        _ => return None,
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Stage {
    Attack,
    Decay,
    Sustain,
    Release,
}
#[derive(Clone, Copy, Debug)]
struct Voice {
    active: bool,
    channel: u8,
    key: u8,
    /// The key is down (else only the pedal or the release keeps it).
    down: bool,
    /// Released while the sustain pedal was down.
    pedal: bool,
    gain: f32,
    patch: Patch,
    drum: Option<Drum>,
    stage: Stage,
    level: f32,
    /// Attack increment per sample, and the decay and release factors.
    rise: f32,
    fall: f32,
    release: f32,
    /// Oscillator phases, 32-bit fractions of a cycle.
    phase: [u32; 2],
    filter: f32,
    /// Samples since the note began (drum sweeps, filter sweeps).
    age: u32,
    started: u64,
    noise: u32,
}
const SILENT: Voice = Voice {
    active: false,
    channel: 0,
    key: 0,
    down: false,
    pedal: false,
    gain: 0.0,
    patch: FAMILIES[0],
    drum: None,
    stage: Stage::Release,
    level: 0.0,
    rise: 0.0,
    fall: 1.0,
    release: 0.0,
    phase: [0; 2],
    filter: 0.0,
    age: 0,
    started: 0,
    noise: 0x1234_5678,
};

#[derive(Clone, Copy, Debug)]
struct Channel {
    program: u8,
    bank: u8,
    volume: u8,
    expression: u8,
    pan: u8,
    modulation: u8,
    sustain: bool,
    /// -8192..=8191.
    bend: i16,
    /// Pitch bend sensitivity in semitones (RPN 0).
    range: u8,
    /// The selected RPN (MSB, LSB); 0x7F7F is none.
    rpn: u16,
}
const fn channel() -> Channel {
    Channel {
        program: 0,
        bank: 0,
        volume: 100,
        expression: 127,
        pan: 64,
        modulation: 0,
        sustain: false,
        bend: 0,
        range: 2,
        rpn: 0x7F7F,
    }
}

pub struct Synth {
    channels: [Channel; CHANNELS],
    voices: [Voice; VOICES],
    /// Master volume (GM sysex), 0..=16383.
    master: u16,
    clock: u64,
    sine: Vec<f32>,
    /// Running status of a byte stream (for `bytes`).
    running: u8,
}
impl Default for Synth {
    fn default() -> Self {
        Self::new()
    }
}
impl Synth {
    pub fn new() -> Self {
        // One sine cycle by rotation (cos/sin of the step by series).
        let step = core::f64::consts::TAU / TABLE as f64;
        let s2 = step * step;
        let (c, s) = (
            1.0 - s2 / 2.0 + s2 * s2 / 24.0 - s2 * s2 * s2 / 720.0,
            step * (1.0 - s2 / 6.0 + s2 * s2 / 120.0 - s2 * s2 * s2 / 5040.0),
        );
        let mut sine = Vec::with_capacity(TABLE + 1);
        let (mut x, mut y) = (1.0f64, 0.0f64);
        for _ in 0..=TABLE {
            sine.push(y as f32);
            (x, y) = (x * c - y * s, x * s + y * c);
        }
        Self {
            channels: [channel(); CHANNELS],
            voices: [SILENT; VOICES],
            master: 16383,
            clock: 0,
            sine,
            running: 0,
        }
    }
    /// Voices sounding.
    pub fn active(&self) -> usize {
        self.voices.iter().filter(|v| v.active).count()
    }
    /// A channel's program (bank select is kept but GM has one bank).
    pub fn program(&self, channel: usize) -> u8 {
        self.channels[channel].program
    }
    /// GM/GS reset: every channel and voice as at power-on.
    pub fn reset(&mut self) {
        self.channels = [channel(); CHANNELS];
        self.voices = [SILENT; VOICES];
        self.master = 16383;
        self.running = 0;
    }
    /// One short message (status, data1, data2).
    pub fn short(&mut self, status: u8, a: u8, b: u8) {
        if status < 0x80 {
            return;
        }
        let ch = usize::from(status & 0x0F);
        let (a, b) = (a & 0x7F, b & 0x7F);
        match status & 0xF0 {
            0x80 => self.note_off(ch, a),
            0x90 if b == 0 => self.note_off(ch, a),
            0x90 => self.note_on(ch, a, b),
            0xB0 => self.control(ch, a, b),
            0xC0 => self.channels[ch].program = a,
            0xE0 => self.channels[ch].bend = ((i16::from(b) << 7) | i16::from(a)) - 8192,
            // Aftertouch and channel pressure have no effect here.
            _ => {}
        }
    }
    /// A packed midiOutShortMsg word (status in the low byte).
    pub fn message(&mut self, word: u32) {
        let [status, a, b, _] = word.to_le_bytes();
        let status = if status >= 0x80 {
            if status < 0xF0 {
                self.running = status;
            }
            status
        } else {
            // Running status: the first byte is data.
            return self.short(self.running, status, a);
        };
        self.short(status, a, b);
    }
    /// A system exclusive message (F0 .. F7): the resets and master volume.
    pub fn sysex(&mut self, data: &[u8]) {
        match data {
            // GM System On / GM2.
            [0xF0, 0x7E, _, 0x09, 0x01 | 0x03, 0xF7] => self.reset(),
            // GS reset (and "system mode set").
            [0xF0, 0x41, _, 0x42, 0x12, 0x40, 0x00, 0x7F, _, _, 0xF7] => self.reset(),
            // XG System On.
            [0xF0, 0x43, _, 0x4C, 0x00, 0x00, 0x7E, 0x00, 0xF7] => self.reset(),
            // Master volume (universal real time).
            [0xF0, 0x7F, _, 0x04, 0x01, lsb, msb, 0xF7] => {
                self.master = (u16::from(*msb & 0x7F) << 7) | u16::from(*lsb & 0x7F);
            }
            _ => {}
        }
    }
    fn control(&mut self, ch: usize, number: u8, value: u8) {
        let c = &mut self.channels[ch];
        match number {
            0 => c.bank = value,
            1 => c.modulation = value,
            6 if c.rpn == 0 => c.range = value.min(24),
            7 => c.volume = value,
            10 => c.pan = value,
            11 => c.expression = value,
            64 => {
                c.sustain = value >= 64;
                if !c.sustain {
                    for v in self.voices.iter_mut() {
                        if v.active && usize::from(v.channel) == ch && v.pedal {
                            v.pedal = false;
                            v.stage = Stage::Release;
                        }
                    }
                }
            }
            100 => c.rpn = (c.rpn & 0x7F00) | u16::from(value),
            101 => c.rpn = (c.rpn & 0x00FF) | (u16::from(value) << 8),
            // All sound off: silence at once.
            120 => {
                for v in self.voices.iter_mut() {
                    if usize::from(v.channel) == ch {
                        v.active = false;
                    }
                }
            }
            121 => {
                let program = c.program;
                let bank = c.bank;
                *c = channel();
                c.program = program;
                c.bank = bank;
            }
            // All notes off (and the omni/mono/poly modes, which imply it).
            123..=127 => {
                for v in self.voices.iter_mut() {
                    if v.active && usize::from(v.channel) == ch {
                        v.down = false;
                        if self.channels[ch].sustain {
                            v.pedal = true;
                        } else {
                            v.stage = Stage::Release;
                        }
                    }
                }
            }
            _ => {}
        }
    }
    fn note_on(&mut self, ch: usize, key: u8, velocity: u8) {
        // A repeated key retriggers its voice.
        let slot = self
            .voices
            .iter()
            .position(|v| v.active && usize::from(v.channel) == ch && v.key == key)
            .or_else(|| self.voices.iter().position(|v| !v.active))
            .unwrap_or_else(|| {
                // Steal: the oldest released voice, else the oldest.
                let released = self
                    .voices
                    .iter()
                    .enumerate()
                    .filter(|(_, v)| v.stage == Stage::Release)
                    .min_by_key(|(_, v)| v.started)
                    .map(|(i, _)| i);
                released.unwrap_or_else(|| {
                    self.voices
                        .iter()
                        .enumerate()
                        .min_by_key(|(_, v)| v.started)
                        .map(|(i, _)| i)
                        .unwrap_or(0)
                })
            });
        let percussion = ch == DRUMS;
        let drum_sound = if percussion {
            match drum(key) {
                Some(d) => Some(d),
                None => return,
            }
        } else {
            None
        };
        let patch = program_patch(self.channels[ch].program);
        // Velocity: a square law, as GM synthesizers respond.
        let v = f32::from(velocity) / 127.0;
        let ms_rise = if drum_sound.is_some() {
            1.0
        } else {
            patch.attack.max(1.0)
        };
        let (fall, release) = match drum_sound {
            Some(Drum::Tone { ms, .. }) | Some(Drum::Noise { ms, .. }) => {
                (decay_factor(ms), decay_factor(ms))
            }
            None => (decay_factor(patch.decay), decay_factor(patch.release)),
        };
        self.clock += 1;
        let previous = self.voices[slot];
        self.voices[slot] = Voice {
            active: true,
            channel: ch as u8,
            key,
            down: true,
            pedal: false,
            gain: v * v,
            patch,
            drum: drum_sound,
            stage: Stage::Attack,
            // A retriggered voice starts from where it was (no click).
            level: if previous.active && previous.key == key {
                previous.level
            } else {
                0.0
            },
            rise: 1.0 / (ms_rise * RATE as f32 / 1000.0),
            fall,
            release,
            phase: [0, 0x4000_0000],
            filter: 0.0,
            age: 0,
            started: self.clock,
            noise: 0x9E37_79B9 ^ (u32::from(key) << 8) ^ self.clock as u32,
        };
    }
    fn note_off(&mut self, ch: usize, key: u8) {
        let sustain = self.channels[ch].sustain;
        for v in self.voices.iter_mut() {
            if v.active && v.down && usize::from(v.channel) == ch && v.key == key {
                v.down = false;
                // Drums ring out whatever the key does.
                if v.drum.is_some() {
                    continue;
                }
                if sustain {
                    v.pedal = true;
                } else {
                    v.stage = Stage::Release;
                }
            }
        }
    }
    fn lookup(&self, phase: u32) -> f32 {
        let at = (phase >> 22) as usize;
        let frac = (phase & 0x003F_FFFF) as f32 / 4_194_304.0;
        let (a, b) = (self.sine[at], self.sine[at + 1]);
        a + (b - a) * frac
    }
    fn oscillator(&self, shape: Shape, phase: u32, noise: &mut u32) -> f32 {
        let x = phase as f32 / 4_294_967_296.0;
        match shape {
            Sine => self.lookup(phase),
            Triangle => {
                if x < 0.5 {
                    4.0 * x - 1.0
                } else {
                    3.0 - 4.0 * x
                }
            }
            Square => {
                if x < 0.5 {
                    0.7
                } else {
                    -0.7
                }
            }
            Pulse => {
                if x < 0.25 {
                    0.7
                } else {
                    -0.7
                }
            }
            Saw => 2.0 * x - 1.0,
            Noise => {
                *noise ^= *noise << 13;
                *noise ^= *noise >> 17;
                *noise ^= *noise << 5;
                (*noise as f32 / 2_147_483_648.0) - 1.0
            }
        }
    }
    /// Adds `frames` of interleaved stereo to `out` (as f32, unscaled).
    fn render_block(&mut self, out: &mut [f32]) {
        let frames = out.len() / 2;
        let master = f32::from(self.master) / 16383.0;
        for i in 0..VOICES {
            let mut v = self.voices[i];
            if !v.active {
                continue;
            }
            let c = self.channels[usize::from(v.channel)];
            let loudness = {
                let vol = f32::from(c.volume) / 127.0;
                let expr = f32::from(c.expression) / 127.0;
                vol * vol * expr * expr * v.gain * master
            };
            // Pan: -3 dB in the centre (sine law through the table).
            let pan = f32::from(c.pan.min(127)) / 127.0;
            let quarter = (pan * 4_294_967_296.0 / 4.0) as u32;
            let (left, right) = (
                self.lookup(0x4000_0000u32.wrapping_add(quarter)),
                self.lookup(quarter),
            );
            // Pitch: key, bend and vibrato (modulation: up to 50 cents at 5 Hz).
            let bend = f32::from(c.bend) / 8192.0 * f32::from(c.range);
            let vibrato = if c.modulation > 0 {
                let t = (v.age as u64 * 5 * 4_294_967_296 / u64::from(RATE)) as u32;
                self.lookup(t) * 0.5 * f32::from(c.modulation) / 127.0
            } else {
                0.0
            };
            let note = f32::from(v.key) + bend + vibrato + v.patch.octave * 12.0;
            let mut steps = [0u32; 2];
            let tone = match v.drum {
                Some(Drum::Tone { from, to, ms }) => {
                    // Exponential sweep from `from` to `to` over a quarter
                    // of the decay.
                    let t = (v.age as f32 / (ms * RATE as f32 / 4000.0)).min(1.0);
                    let hz = from + (to - from) * (1.0 - exp2(-6.0 * t)) / (1.0 - exp2(-6.0));
                    steps[0] = (hz / RATE as f32 * 4_294_967_296.0) as u32;
                    hz
                }
                Some(Drum::Noise { tone, .. }) => {
                    steps[0] = (tone / RATE as f32 * 4_294_967_296.0) as u32;
                    tone
                }
                None => {
                    let hz = frequency(note);
                    steps[0] = (hz / RATE as f32 * 4_294_967_296.0) as u32;
                    let second = hz * exp2(v.patch.detune / 1200.0);
                    steps[1] = (second / RATE as f32 * 4_294_967_296.0) as u32;
                    hz
                }
            };
            // The low-pass coefficient: the cutoff follows the note, opened
            // by brightness, swept down from the attack.
            let sweep = 1.0 + v.patch.sweep * exp2(-(v.age as f32) / (RATE as f32 * 0.08)) * 4.0;
            let cutoff = (tone.max(80.0) * (1.0 + 12.0 * v.patch.bright) * sweep).min(18_000.0);
            let alpha = (cutoff * core::f32::consts::TAU / RATE as f32).min(1.0);
            for f in 0..frames {
                // Envelope.
                match v.stage {
                    Stage::Attack => {
                        v.level += v.rise;
                        if v.level >= 1.0 {
                            v.level = 1.0;
                            v.stage = Stage::Decay;
                        }
                    }
                    Stage::Decay => {
                        let floor = if v.drum.is_some() {
                            0.0
                        } else {
                            v.patch.sustain
                        };
                        v.level = floor + (v.level - floor) * v.fall;
                        if v.patch.decay <= 0.0 || v.level - floor < 1e-4 {
                            v.level = floor;
                            v.stage = Stage::Sustain;
                        }
                    }
                    Stage::Sustain => {}
                    Stage::Release => v.level *= v.release,
                }
                if (v.stage == Stage::Release || v.stage == Stage::Sustain) && v.level < 1e-4 {
                    v.active = false;
                    break;
                }
                let sample = match v.drum {
                    Some(Drum::Tone { .. }) => self.lookup(v.phase[0]),
                    Some(Drum::Noise { high, tone, .. }) => {
                        let n = self.oscillator(Noise, 0, &mut v.noise);
                        v.filter += 0.25 * (n - v.filter);
                        let body = if high { n - v.filter } else { v.filter };
                        let ring = if tone > 0.0 {
                            self.lookup(v.phase[0]) * 0.5
                        } else {
                            0.0
                        };
                        body + ring
                    }
                    None => {
                        let a = self.oscillator(v.patch.shape, v.phase[0], &mut v.noise);
                        let b = self.oscillator(v.patch.second, v.phase[1], &mut v.noise);
                        let x = a * (1.0 - v.patch.mix) + b * v.patch.mix;
                        v.filter += alpha * (x - v.filter);
                        v.filter
                    }
                };
                v.phase[0] = v.phase[0].wrapping_add(steps[0]);
                v.phase[1] = v.phase[1].wrapping_add(steps[1]);
                v.age = v.age.saturating_add(1);
                let s = sample * v.level * loudness * 0.8;
                out[2 * f] += s * left;
                out[2 * f + 1] += s * right;
            }
            self.voices[i] = v;
        }
    }
    /// Renders `out.len() / 2` stereo frames of 16-bit PCM.
    pub fn render(&mut self, out: &mut [i16]) {
        let mut mix = [0f32; BLOCK * 2];
        for chunk in out.chunks_mut(BLOCK * 2) {
            let part = &mut mix[..chunk.len()];
            part.fill(0.0);
            self.render_block(part);
            for (o, x) in chunk.iter_mut().zip(part.iter()) {
                // A soft knee above three quarters of full scale.
                let y = if *x > 0.75 {
                    0.75 + 0.25 * (1.0 - exp2(-(*x - 0.75) * 5.77))
                } else if *x < -0.75 {
                    -0.75 - 0.25 * (1.0 - exp2((*x + 0.75) * 5.77))
                } else {
                    *x
                };
                *o = (y * 32767.0) as i16;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    /// The dominant frequency of the left channel by counting upward zero
    /// crossings.
    fn pitch(pcm: &[i16]) -> f32 {
        let left: Vec<i32> = pcm.iter().step_by(2).map(|s| i32::from(*s)).collect();
        let crossings = left.windows(2).filter(|w| w[0] < 0 && w[1] >= 0).count();
        crossings as f32 * RATE as f32 / left.len() as f32
    }
    fn render(s: &mut Synth, seconds: f32) -> Vec<i16> {
        let mut pcm = alloc::vec![0i16; (seconds * RATE as f32) as usize * 2];
        s.render(&mut pcm);
        pcm
    }
    #[test]
    fn exp2_and_note_frequencies() {
        for (x, want) in [
            (0.0f32, 1.0f32),
            (1.0, 2.0),
            (-1.0, 0.5),
            (0.5, core::f32::consts::SQRT_2),
            (-3.25, 0.105_112_1),
        ] {
            assert!((exp2(x) - want).abs() < want * 1e-5, "2^{x}");
        }
        assert!((frequency(69.0) - 440.0).abs() < 0.01);
        assert!((frequency(81.0) - 880.0).abs() < 0.02);
        assert!((frequency(60.0) - 261.626).abs() < 0.01);
    }
    #[test]
    fn a_square_lead_sounds_at_its_pitch_and_stops() {
        let mut s = Synth::new();
        s.message(0x0000_50C0); // program 80 (square lead), channel 1
        s.message(0x006E_4590); // A4 on
        let pcm = render(&mut s, 0.5);
        let p = pitch(&pcm[2 * 4800..]);
        assert!((p - 440.0).abs() < 6.0, "pitch {p}");
        let peak = pcm.iter().map(|v| v.unsigned_abs()).max().unwrap();
        assert!(peak > 5000, "peak {peak}");
        s.message(0x0000_4580); // A4 off
        let tail = render(&mut s, 0.5);
        assert_eq!(s.active(), 0);
        assert!(tail[tail.len() - 200..].iter().all(|v| *v == 0));
    }
    #[test]
    fn running_status_bend_and_volume() {
        let mut s = Synth::new();
        s.message(0x0000_50C0);
        s.message(0x0064_3C90); // C4 on
        s.message(0x0064_4000); // running status: E4 on (0x40 is data)
        assert_eq!(s.active(), 2);
        // Bend fully up (two semitones by default): A4 sounds as B4.
        let mut b = Synth::new();
        b.message(0x0000_50C0);
        b.message(0x007F_7FE0);
        b.message(0x006E_4590);
        let p = pitch(&render(&mut b, 0.5)[9600..]);
        assert!((p - 493.9).abs() < 8.0, "bent pitch {p}");
        // Channel volume 0 is silence.
        let mut q = Synth::new();
        q.message(0x0000_07B0);
        q.message(0x006E_4590);
        assert!(render(&mut q, 0.2).iter().all(|v| *v == 0));
    }
    #[test]
    fn sustain_pedal_holds_released_notes() {
        let mut s = Synth::new();
        s.message(0x0000_10C0); // organ: sustains while held
        s.message(0x007F_40B0); // pedal down
        s.message(0x006E_4590);
        render(&mut s, 0.1);
        s.message(0x0000_4580); // key up: the pedal keeps it
        render(&mut s, 0.5);
        assert_eq!(s.active(), 1);
        s.message(0x0000_40B0); // pedal up: released
        render(&mut s, 1.0);
        assert_eq!(s.active(), 0);
    }
    #[test]
    fn drums_and_resets() {
        let mut s = Synth::new();
        s.message(0x0064_2499); // kick on channel 10
        s.message(0x0064_2A99); // closed hat
        let pcm = render(&mut s, 0.1);
        assert!(pcm.iter().any(|v| v.unsigned_abs() > 1000));
        // An unmapped percussion key makes no voice.
        let before = s.active();
        s.message(0x0064_1099);
        assert_eq!(s.active(), before);
        s.message(0x0000_05C3);
        assert_eq!(s.program(3), 5);
        s.sysex(&[0xF0, 0x7E, 0x7F, 0x09, 0x01, 0xF7]);
        assert_eq!(s.active(), 0);
        assert_eq!(s.program(3), 0);
        // Master volume zero silences new notes.
        s.sysex(&[0xF0, 0x7F, 0x7F, 0x04, 0x01, 0x00, 0x00, 0xF7]);
        s.message(0x006E_4590);
        assert!(render(&mut s, 0.1).iter().all(|v| *v == 0));
    }
    #[test]
    fn thirty_two_voices_steal_the_oldest() {
        let mut s = Synth::new();
        s.message(0x0000_10C0);
        for key in 30..70u32 {
            s.message(0x0050_0090 | key << 8);
        }
        assert_eq!(s.active(), VOICES);
        let pcm = render(&mut s, 0.05);
        assert!(pcm.iter().all(|v| v.unsigned_abs() < 32767), "clipped");
    }
}
