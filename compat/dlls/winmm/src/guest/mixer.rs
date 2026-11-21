//! The mixer API (mixer*): one mixer per endpoint, as Windows 11 builds
//! them from the audio endpoints (measured on the reference PC). The
//! speakers' mixer has the master volume line (the endpoint's volume and
//! mute, door.audio's master output) with two sources, CD Audio and the
//! wave output (this application's volume, as waveOutSetVolume sets it);
//! the microphone's has its master line and the microphone source.
//! The microphone's volume and mute (its master line and the microphone
//! source alike) are door.audio's input level, which Settings > System >
//! Sound > Input > Volume shows and the shell keeps.
use super::*;
use door_service_protocol::audio::{Input, Output};

const MIXERR_INVALLINE: u32 = 1024;
const MIXERR_INVALCONTROL: u32 = 1025;
const MIXERR_INVALVALUE: u32 = 1026;
const OBJECTF_HANDLE: u32 = 0x8000_0000;
const OBJECTF_TYPEMASK: u32 = 0xF000_0000;
const OBJECTF_MIXER: u32 = 0x0000_0000;
const OBJECTF_WAVEOUT: u32 = 0x1000_0000;
const OBJECTF_WAVEIN: u32 = 0x2000_0000;
const OBJECTF_MIDIOUT: u32 = 0x3000_0000;
const LINE_DESTINATION: u32 = 0;
const LINE_SOURCE: u32 = 1;
const LINE_ID: u32 = 2;
const LINE_COMPONENTTYPE: u32 = 3;
const LINE_TARGETTYPE: u32 = 4;
const CONTROLS_ALL: u32 = 0;
const CONTROLS_ONEBYID: u32 = 1;
const CONTROLS_ONEBYTYPE: u32 = 2;
const DETAILS_VALUE: u32 = 0;
const MUTE: u32 = 0x2001_0002;
const VOLUME: u32 = 0x5003_0001;
/// MIXERLINE_LINEF_ACTIVE and _SOURCE.
const ACTIVE: u32 = 0x1;
const SOURCE: u32 = 0x8000_0000;
const DESTINATION_LINE: u32 = 0xFFFF_0000;
/// MIXERLINE_TARGETTYPE_*.
const TARGET_UNDEFINED: u32 = 0;
const TARGET_WAVEOUT: u32 = 1;
const TARGET_WAVEIN: u32 = 2;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mixer {
    Speakers,
    Microphone,
}
impl Mixer {
    fn name(self) -> &'static str {
        match self {
            Self::Speakers => "Speakers (VirtIO Sound)",
            Self::Microphone => "Microphone (VirtIO Sound)",
        }
    }
}
/// The mixers present, in device order.
fn mixers() -> Vec<Mixer> {
    let mut m = Vec::new();
    if device_present() {
        m.push(Mixer::Speakers);
    }
    if capture_present() {
        m.push(Mixer::Microphone);
    }
    m
}
struct Line {
    id: u32,
    source: u32,
    flags: u32,
    component: u32,
    connections: u32,
    /// (mute control id, volume control id).
    controls: (u32, u32),
    short: &'static str,
    name: &'static str,
    /// (type, wMid, wPid, the device's name or none).
    target: (u32, u16, u16, bool),
}
fn lines(mixer: Mixer) -> Vec<Line> {
    match mixer {
        Mixer::Speakers => alloc::vec![
            Line {
                id: DESTINATION_LINE,
                source: u32::MAX,
                flags: ACTIVE,
                component: 4,
                connections: 2,
                controls: (1, 2),
                short: "Volume",
                name: "Master Volume",
                target: (TARGET_UNDEFINED, 65535, 65535, false),
            },
            Line {
                id: 0,
                source: 0,
                flags: SOURCE | ACTIVE,
                component: 0x1005,
                connections: 0,
                controls: (3, 4),
                short: "CDAudio",
                name: "CD Audio",
                target: (TARGET_UNDEFINED, 1, 65535, true),
            },
            Line {
                id: 0x0001_0000,
                source: 1,
                flags: SOURCE | ACTIVE,
                component: 0x1008,
                connections: 0,
                controls: (5, 6),
                short: "Volume",
                name: "Master Volume",
                target: (TARGET_WAVEOUT, 1, 65535, true),
            },
        ],
        Mixer::Microphone => alloc::vec![
            Line {
                id: DESTINATION_LINE,
                source: u32::MAX,
                flags: ACTIVE,
                component: 7,
                connections: 1,
                controls: (1, 2),
                short: "Volume",
                name: "Master Volume",
                target: (TARGET_WAVEIN, 1, 65535, true),
            },
            Line {
                id: 0,
                source: 0,
                flags: SOURCE | ACTIVE,
                component: 0x1003,
                connections: 0,
                controls: (3, 4),
                short: "Volume",
                name: "Master Volume",
                target: (TARGET_UNDEFINED, 65535, 65535, false),
            },
        ],
    }
}

// ---- handles and objects -------------------------------------------------------
static HANDLES: Lock<Vec<(u64, usize)>> = Lock::new(Vec::new());
static NEXT_MIXER: AtomicU64 = AtomicU64::new(0x00C0_0010);
/// The mixer an object names (MIXER_OBJECTF_*).
fn resolve(object: u64, flags: u32) -> Result<(usize, Mixer), u32> {
    let present = mixers();
    let index = match flags & OBJECTF_TYPEMASK {
        f if f == OBJECTF_MIXER => object as usize,
        f if f == OBJECTF_MIXER | OBJECTF_HANDLE => HANDLES
            .with(|h| {
                h.iter()
                    .find(|(handle, _)| *handle == object)
                    .map(|(_, i)| *i)
            })
            .ok_or(mm::INVALHANDLE)?,
        // Wave output (an ID or a handle) and MIDI output play through the
        // speakers; wave input records from the microphone.
        f if f == OBJECTF_WAVEOUT
            || f == OBJECTF_WAVEOUT | OBJECTF_HANDLE
            || f == OBJECTF_MIDIOUT
            || f == OBJECTF_MIDIOUT | OBJECTF_HANDLE =>
        {
            if f & OBJECTF_HANDLE == 0 && object != 0 && object != u64::from(u32::MAX) {
                return Err(mm::BADDEVICEID);
            }
            present
                .iter()
                .position(|m| *m == Mixer::Speakers)
                .ok_or(mm::NODRIVER)?
        }
        f if f == OBJECTF_WAVEIN || f == OBJECTF_WAVEIN | OBJECTF_HANDLE => {
            if f & OBJECTF_HANDLE == 0 && object != 0 && object != u64::from(u32::MAX) {
                return Err(mm::BADDEVICEID);
            }
            present
                .iter()
                .position(|m| *m == Mixer::Microphone)
                .ok_or(mm::NODRIVER)?
        }
        _ => return Err(mm::INVALFLAG),
    };
    present
        .get(index)
        .map(|m| (index, *m))
        .ok_or(mm::BADDEVICEID)
}

// ---- exports --------------------------------------------------------------------
#[unsafe(no_mangle)]
pub extern "system" fn mixerGetNumDevs() -> u32 {
    mixers().len() as u32
}
fn text(out: &mut Vec<u8>, s: &str, units: usize, wide: bool) {
    if wide {
        let mut b = alloc::vec![0u8; units * 2];
        for (i, u) in s.encode_utf16().take(units - 1).enumerate() {
            b[i * 2..i * 2 + 2].copy_from_slice(&u.to_le_bytes());
        }
        out.extend_from_slice(&b);
    } else {
        let mut b = alloc::vec![0u8; units];
        let n = s.len().min(units - 1);
        b[..n].copy_from_slice(&s.as_bytes()[..n]);
        out.extend_from_slice(&b);
    }
}
/// MIXERCAPS: wMid, wPid, vDriverVersion, szPname[32], fdwSupport,
/// cDestinations (as Windows reports an endpoint's mixer).
fn caps(device: u64, out: u64, size: u32, wide: bool) -> u32 {
    let result = (|| {
        let (_, mixer) = resolve(device, OBJECTF_MIXER)?;
        let mut b = Vec::new();
        b.extend_from_slice(&1u16.to_le_bytes());
        b.extend_from_slice(&65535u16.to_le_bytes());
        b.extend_from_slice(&0u32.to_le_bytes());
        text(&mut b, mixer.name(), 32, wide);
        b.extend_from_slice(&0u32.to_le_bytes());
        b.extend_from_slice(&1u32.to_le_bytes());
        let n = (size as usize).min(b.len());
        write(out, &b[..n])
    })();
    result.err().unwrap_or(mm::NOERROR)
}
#[unsafe(no_mangle)]
pub extern "system" fn mixerGetDevCapsW(device: usize, out: usize, size: u32) -> u32 {
    let device = door_windows_abi::widen(device);
    let out = door_windows_abi::widen(out);
    caps(device, out, size, true)
}
#[unsafe(no_mangle)]
pub extern "system" fn mixerGetDevCapsA(device: usize, out: usize, size: u32) -> u32 {
    let device = door_windows_abi::widen(device);
    let out = door_windows_abi::widen(out);
    caps(device, out, size, false)
}
#[unsafe(no_mangle)]
pub extern "system" fn mixerOpen(
    out: usize,
    device: u32,
    _callback: usize,
    _instance: usize,
    flags: u32,
) -> u32 {
    let out = door_windows_abi::widen(out);
    {
        let result = (|| {
            if out == 0 {
                return Err(mm::INVALPARAM);
            }
            // Change notifications (CALLBACK_WINDOW) are not sent: Doors'
            // master volume changes come from the shell.
            let (index, _) = resolve(u64::from(device), flags & OBJECTF_TYPEMASK)?;
            let handle = NEXT_MIXER.fetch_add(0x10, Ordering::Relaxed);
            HANDLES.with(|h| h.push((handle, index)));
            super::write_pointer(out, handle)
        })();
        result.err().unwrap_or(mm::NOERROR)
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn mixerClose(handle: usize) -> u32 {
    let handle = door_windows_abi::widen(handle);
    {
        let removed = HANDLES.with(|h| {
            let before = h.len();
            h.retain(|(x, _)| *x != handle);
            before != h.len()
        });
        if removed {
            mm::NOERROR
        } else {
            mm::INVALHANDLE
        }
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn mixerGetID(object: usize, out: usize, flags: u32) -> u32 {
    let object = door_windows_abi::widen(object);
    let out = door_windows_abi::widen(out);
    {
        resolve(object, flags)
            .and_then(|(index, _)| write_u32(out, index as u32))
            .err()
            .unwrap_or(mm::NOERROR)
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn mixerMessage(_mixer: usize, _message: u32, _a: usize, _b: usize) -> u32 {
    mm::NOTSUPPORTED
}
/// MIXERLINE (mmsystem.h packs it): cbStruct 0, dwDestination 4, dwSource
/// 8, dwLineID 12, fdwLine 16, dwUser 20 (a DWORD_PTR, P bytes), then
/// dwComponentType, cChannels, cConnections, cControls, szShortName,
/// szName, Target (x64 sizes; 4 bytes less on x86).
const P: usize = super::POINTER;
const NARROWER: u32 = (8 - P) as u32;
fn line_info(object: u64, out: u64, flags: u32, wide: bool) -> u32 {
    let result = (|| {
        let (_, mixer) = resolve(object, flags & OBJECTF_TYPEMASK)?;
        let size = read_u32(out)?;
        let full = if wide { 284 } else { 172 } - NARROWER;
        if size < full {
            return Err(mm::INVALPARAM);
        }
        let all = lines(mixer);
        let line = match flags & 0xF {
            LINE_DESTINATION => {
                if read_u32(out + 4)? != 0 {
                    return Err(mm::INVALPARAM);
                }
                all.first()
            }
            LINE_SOURCE => {
                if read_u32(out + 4)? != 0 {
                    return Err(mm::INVALPARAM);
                }
                let source = read_u32(out + 8)?;
                all.iter()
                    .find(|l| l.source == source && l.flags & SOURCE != 0)
            }
            LINE_ID => {
                let id = read_u32(out + 12)?;
                all.iter().find(|l| l.id == id)
            }
            LINE_COMPONENTTYPE => {
                let component = read_u32(out + 20 + P as u64)?;
                all.iter().find(|l| l.component == component)
            }
            LINE_TARGETTYPE => {
                let target = read_u32(out + u64::from(if wide { 204 } else { 124 } - NARROWER))?;
                all.iter()
                    .find(|l| l.target.0 == target && target != TARGET_UNDEFINED)
            }
            _ => return Err(mm::INVALFLAG),
        }
        .ok_or(MIXERR_INVALLINE)?;
        let mut b = Vec::with_capacity(full as usize);
        b.extend_from_slice(&size.to_le_bytes());
        b.extend_from_slice(&0u32.to_le_bytes());
        b.extend_from_slice(&line.source.to_le_bytes());
        b.extend_from_slice(&line.id.to_le_bytes());
        b.extend_from_slice(&line.flags.to_le_bytes());
        b.extend_from_slice(&[0u8; P]);
        b.extend_from_slice(&line.component.to_le_bytes());
        b.extend_from_slice(&1u32.to_le_bytes());
        b.extend_from_slice(&line.connections.to_le_bytes());
        b.extend_from_slice(&2u32.to_le_bytes());
        text(&mut b, line.short, 16, wide);
        text(&mut b, line.name, 64, wide);
        let (kind, mid, pid, named) = line.target;
        b.extend_from_slice(&kind.to_le_bytes());
        b.extend_from_slice(&0u32.to_le_bytes());
        b.extend_from_slice(&mid.to_le_bytes());
        b.extend_from_slice(&pid.to_le_bytes());
        b.extend_from_slice(&0u32.to_le_bytes());
        text(&mut b, if named { mixer.name() } else { "" }, 32, wide);
        write(out, &b)
    })();
    result.err().unwrap_or(mm::NOERROR)
}
#[unsafe(no_mangle)]
pub extern "system" fn mixerGetLineInfoW(object: usize, out: usize, flags: u32) -> u32 {
    let object = door_windows_abi::widen(object);
    let out = door_windows_abi::widen(out);
    line_info(object, out, flags, true)
}
#[unsafe(no_mangle)]
pub extern "system" fn mixerGetLineInfoA(object: usize, out: usize, flags: u32) -> u32 {
    let object = door_windows_abi::widen(object);
    let out = door_windows_abi::widen(out);
    line_info(object, out, flags, false)
}
/// One MIXERCONTROL: cbStruct, dwControlID, dwControlType, fdwControl
/// (uniform), cMultipleItems, names, Bounds (min, max), Metrics (steps).
fn control(id: u32, kind: u32, wide: bool) -> Vec<u8> {
    let mut b = Vec::new();
    let size: u32 = if wide { 228 } else { 148 };
    b.extend_from_slice(&size.to_le_bytes());
    b.extend_from_slice(&id.to_le_bytes());
    b.extend_from_slice(&kind.to_le_bytes());
    b.extend_from_slice(&1u32.to_le_bytes());
    b.extend_from_slice(&0u32.to_le_bytes());
    let name = if kind == MUTE { "Mute" } else { "Volume" };
    text(&mut b, name, 16, wide);
    text(&mut b, name, 64, wide);
    let (max, steps) = if kind == MUTE {
        (1u32, 0u32)
    } else {
        (65535, 192)
    };
    let mut bounds = [0u8; 24];
    bounds[4..8].copy_from_slice(&max.to_le_bytes());
    b.extend_from_slice(&bounds);
    let mut metrics = [0u8; 24];
    metrics[..4].copy_from_slice(&steps.to_le_bytes());
    b.extend_from_slice(&metrics);
    b
}
/// MIXERLINECONTROLS (packed): cbStruct 0, dwLineID 4, dwControlID/Type 8,
/// cControls 12, cbmxctrl 16, pamxctrl 20 (28 bytes on x64, 24 on x86).
fn line_controls(object: u64, out: u64, flags: u32, wide: bool) -> u32 {
    let result = (|| {
        let (_, mixer) = resolve(object, flags & OBJECTF_TYPEMASK)?;
        let mut head = [0u8; 20 + P];
        read(out, &mut head)?;
        let word = |i: usize| u32::from_le_bytes(head[i..i + 4].try_into().unwrap());
        let (line_id, which, count, each) = (word(4), word(8), word(12), word(16));
        let array = door_windows_abi::layout::get(
            &head[20..],
            &[door_windows_abi::layout::Field::Pointer],
            0,
        );
        let full = if wide { 228 } else { 148 };
        if (word(0) as usize) < 20 + P || each < full {
            return Err(mm::INVALPARAM);
        }
        let all = lines(mixer);
        let chosen: Vec<(u32, u32)> = match flags & 0xF {
            CONTROLS_ALL => {
                let line = all
                    .iter()
                    .find(|l| l.id == line_id)
                    .ok_or(MIXERR_INVALLINE)?;
                if count != 2 {
                    return Err(mm::INVALPARAM);
                }
                alloc::vec![(line.controls.0, MUTE), (line.controls.1, VOLUME)]
            }
            CONTROLS_ONEBYID => {
                let line = all
                    .iter()
                    .find(|l| l.controls.0 == which || l.controls.1 == which)
                    .ok_or(MIXERR_INVALCONTROL)?;
                // The line the control belongs to goes back in dwLineID.
                write_u32(out + 4, line.id)?;
                alloc::vec![(
                    which,
                    if line.controls.0 == which {
                        MUTE
                    } else {
                        VOLUME
                    }
                )]
            }
            CONTROLS_ONEBYTYPE => {
                let line = all
                    .iter()
                    .find(|l| l.id == line_id)
                    .ok_or(MIXERR_INVALLINE)?;
                match which {
                    MUTE => alloc::vec![(line.controls.0, MUTE)],
                    VOLUME => alloc::vec![(line.controls.1, VOLUME)],
                    _ => return Err(MIXERR_INVALCONTROL),
                }
            }
            _ => return Err(mm::INVALFLAG),
        };
        for (i, (id, kind)) in chosen.iter().enumerate() {
            let b = control(*id, *kind, wide);
            write(array + i as u64 * u64::from(each), &b)?;
        }
        Ok(())
    })();
    result.err().unwrap_or(mm::NOERROR)
}
#[unsafe(no_mangle)]
pub extern "system" fn mixerGetLineControlsW(object: usize, out: usize, flags: u32) -> u32 {
    let object = door_windows_abi::widen(object);
    let out = door_windows_abi::widen(out);
    line_controls(object, out, flags, true)
}
#[unsafe(no_mangle)]
pub extern "system" fn mixerGetLineControlsA(object: usize, out: usize, flags: u32) -> u32 {
    let object = door_windows_abi::widen(object);
    let out = door_windows_abi::widen(out);
    line_controls(object, out, flags, false)
}

// ---- values ---------------------------------------------------------------------
/// CD Audio has no player in Doors; its controls keep what they are set to.
static CD: AtomicU32 = AtomicU32::new(0xFFFF);
static CD_MUTE: AtomicBool = AtomicBool::new(false);
/// The wave output's mute keeps the volume it silenced.
static WAVE_MUTED: AtomicU32 = AtomicU32::new(u32::MAX);
fn output() -> Result<Output, u32> {
    audio(|a| a.status().ok().map(|s| s.output))
        .flatten()
        .ok_or(mm::NODRIVER)
}
fn input() -> Result<Input, u32> {
    audio(|a| a.status().ok().map(|s| s.input))
        .flatten()
        .ok_or(mm::NODRIVER)
}
fn word_of(percent: u8) -> u32 {
    (u32::from(percent.min(100)) * 0xFFFF).div_ceil(100)
}
fn get_value(mixer: Mixer, id: u32) -> Result<u32, u32> {
    Ok(match (mixer, id) {
        (Mixer::Speakers, 1) => u32::from(output()?.muted),
        (Mixer::Speakers, 2) => word_of(output()?.volume),
        (Mixer::Speakers, 3) => u32::from(CD_MUTE.load(Ordering::Relaxed)),
        (Mixer::Speakers, 4) => CD.load(Ordering::Relaxed),
        (Mixer::Speakers, 5) => u32::from(WAVE_MUTED.load(Ordering::Relaxed) != u32::MAX),
        (Mixer::Speakers, 6) => {
            let muted = WAVE_MUTED.load(Ordering::Relaxed);
            let v = if muted == u32::MAX {
                APP_VOLUME.load(Ordering::Relaxed)
            } else {
                muted
            };
            ((v & 0xFFFF) + (v >> 16)) / 2
        }
        (Mixer::Microphone, 1 | 3) => u32::from(input()?.muted),
        (Mixer::Microphone, 2 | 4) => word_of(input()?.volume),
        _ => return Err(MIXERR_INVALCONTROL),
    })
}
fn set_value(mixer: Mixer, id: u32, value: u32) -> Result<(), u32> {
    let boolean = matches!(id, 1 | 3 | 5);
    if (boolean && value > 1) || value > 0xFFFF {
        return Err(MIXERR_INVALVALUE);
    }
    match (mixer, id) {
        (Mixer::Speakers, 1 | 2) => {
            let mut o = output()?;
            if id == 1 {
                o.muted = value != 0;
            } else {
                o.volume = ((value * 100 + 0x7FFF) / 0xFFFF) as u8;
            }
            audio(|a| a.set_output(o))
                .ok_or(mm::NODRIVER)?
                .map_err(|_| mm::ERROR)
        }
        (Mixer::Speakers, 3) => {
            CD_MUTE.store(value != 0, Ordering::Relaxed);
            Ok(())
        }
        (Mixer::Speakers, 4) => {
            CD.store(value, Ordering::Relaxed);
            Ok(())
        }
        (Mixer::Speakers, 5) => {
            let muted = WAVE_MUTED.load(Ordering::Relaxed);
            if value != 0 && muted == u32::MAX {
                WAVE_MUTED.store(APP_VOLUME.load(Ordering::Relaxed), Ordering::Relaxed);
                super::waveOutSetVolume(0, 0);
            } else if value == 0 && muted != u32::MAX {
                WAVE_MUTED.store(u32::MAX, Ordering::Relaxed);
                super::waveOutSetVolume(0, muted);
            }
            Ok(())
        }
        (Mixer::Speakers, 6) => {
            let both = value | value << 16;
            if WAVE_MUTED.load(Ordering::Relaxed) != u32::MAX {
                WAVE_MUTED.store(both, Ordering::Relaxed);
            } else {
                super::waveOutSetVolume(0, both);
            }
            Ok(())
        }
        (Mixer::Microphone, 1..=4) => {
            let mut i = input()?;
            if id % 2 == 1 {
                i.muted = value != 0;
            } else {
                i.volume = ((value * 100 + 0x7FFF) / 0xFFFF) as u8;
            }
            audio(|a| a.set_input(i))
                .ok_or(mm::NODRIVER)?
                .map_err(|_| mm::ERROR)
        }
        _ => Err(MIXERR_INVALCONTROL),
    }
}
/// MIXERCONTROLDETAILS (packed): cbStruct 0, dwControlID 4, cChannels 8,
/// hwndOwner/cMultipleItems 12 (P bytes), cbDetails, paDetails (32 bytes on
/// x64, 24 on x86).
fn details(object: u64, details: u64, flags: u32) -> Result<(Mixer, u32, u32, u64), u32> {
    let (_, mixer) = resolve(object, flags & OBJECTF_TYPEMASK)?;
    let mut head = [0u8; 16 + 2 * P];
    read(details, &mut head)?;
    let word = |i: usize| u32::from_le_bytes(head[i..i + 4].try_into().unwrap());
    if (word(0) as usize) < 16 + 2 * P || word(12 + P) < 4 {
        return Err(mm::INVALPARAM);
    }
    let (id, channels) = (word(4), word(8));
    if !(1..=2).contains(&channels) || word(12) != 0 {
        return Err(mm::INVALPARAM);
    }
    let array = door_windows_abi::layout::get(
        &head[16 + P..],
        &[door_windows_abi::layout::Field::Pointer],
        0,
    );
    Ok((mixer, id, channels, array))
}
#[unsafe(no_mangle)]
pub extern "system" fn mixerGetControlDetailsW(object: usize, out: usize, flags: u32) -> u32 {
    let object = door_windows_abi::widen(object);
    let out = door_windows_abi::widen(out);
    {
        let result = (|| {
            if flags & 0xF != DETAILS_VALUE {
                return Err(MIXERR_INVALCONTROL);
            }
            let (mixer, id, channels, array) = details(object, out, flags)?;
            let value = get_value(mixer, id)?;
            let each = read_u32(out + 20)?;
            for c in 0..channels {
                write_u32(array + u64::from(c * each), value)?;
            }
            Ok(())
        })();
        result.err().unwrap_or(mm::NOERROR)
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn mixerGetControlDetailsA(object: usize, out: usize, flags: u32) -> u32 {
    let object = door_windows_abi::widen(object);
    let out = door_windows_abi::widen(out);
    {
        mixerGetControlDetailsW(
            door_windows_abi::narrow(object),
            door_windows_abi::narrow(out),
            flags,
        )
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn mixerSetControlDetails(object: usize, input: usize, flags: u32) -> u32 {
    let object = door_windows_abi::widen(object);
    let input = door_windows_abi::widen(input);
    {
        let result = (|| {
            if flags & 0xF != DETAILS_VALUE {
                return Err(mm::NOTSUPPORTED);
            }
            let (mixer, id, channels, array) = details(object, input, flags)?;
            let each = read_u32(input + 20)?;
            // A uniform control takes the channels' mean.
            let mut sum = 0u64;
            for c in 0..channels {
                sum += u64::from(read_u32(array + u64::from(c * each))?);
            }
            set_value(mixer, id, (sum / u64::from(channels)) as u32)
        })();
        result.err().unwrap_or(mm::NOERROR)
    }
}
