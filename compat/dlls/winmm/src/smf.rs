//! Standard MIDI Files (format 0 and 1; RIFF RMID wrapping too) for the
//! MCI sequencer: the tracks merged into one time-ordered event list, and
//! the tempo map that turns ticks into time.
use alloc::vec::Vec;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    /// A channel message packed as midiOutShortMsg takes it.
    Short(u32),
    /// A system exclusive message, F0 included and F7 terminated.
    Sysex(Vec<u8>),
    /// Microseconds per quarter note.
    Tempo(u32),
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Song {
    /// Ticks per quarter note (SMPTE divisions are converted to an
    /// equivalent at 120 bpm).
    pub division: u16,
    /// (absolute tick, event), in order.
    pub events: Vec<(u64, Event)>,
    /// The last tick (the latest end of track).
    pub end: u64,
}
fn vlq(b: &[u8], at: &mut usize) -> Option<u32> {
    let mut v = 0u32;
    for _ in 0..4 {
        let byte = *b.get(*at)?;
        *at += 1;
        v = (v << 7) | u32::from(byte & 0x7F);
        if byte & 0x80 == 0 {
            return Some(v);
        }
    }
    None
}
fn be32(b: &[u8]) -> u32 {
    u32::from_be_bytes([b[0], b[1], b[2], b[3]])
}
/// Parses a .mid (or RIFF RMID) image.
pub fn parse(mut b: &[u8]) -> Option<Song> {
    if b.len() >= 20 && &b[..4] == b"RIFF" && &b[8..12] == b"RMID" {
        // RIFF RMID: the SMF is the "data" chunk.
        let mut at = 12;
        loop {
            let header = b.get(at..at + 8)?;
            let length = u32::from_le_bytes(header[4..8].try_into().ok()?) as usize;
            if &header[..4] == b"data" {
                b = b.get(at + 8..at + 8 + length)?;
                break;
            }
            at = at.checked_add(8 + length + (length & 1))?;
        }
    }
    if b.len() < 14 || &b[..4] != b"MThd" || be32(&b[4..8]) < 6 {
        return None;
    }
    let format = u16::from_be_bytes([b[8], b[9]]);
    let tracks = u16::from_be_bytes([b[10], b[11]]);
    let raw = u16::from_be_bytes([b[12], b[13]]);
    if format > 1 || tracks == 0 {
        return None;
    }
    let division = if raw & 0x8000 != 0 {
        // SMPTE: frames per second times ticks per frame per second;
        // at the default 500000 us per quarter that is half of it per beat.
        let fps = u16::from(((raw >> 8) as u8 as i8).unsigned_abs());
        let per_frame = raw & 0xFF;
        (fps * per_frame / 2).max(1)
    } else {
        raw.max(1)
    };
    let mut at = 8 + be32(&b[4..8]) as usize;
    let mut events: Vec<(u64, u32, Event)> = Vec::new();
    let mut end = 0u64;
    let mut order = 0u32;
    for _ in 0..tracks {
        let header = b.get(at..at + 8)?;
        let length = be32(&header[4..8]) as usize;
        let body = b.get(at + 8..at + 8 + length)?;
        at += 8 + length;
        if &header[..4] != b"MTrk" {
            continue;
        }
        let (mut i, mut tick, mut running) = (0usize, 0u64, 0u8);
        while i < body.len() {
            tick += u64::from(vlq(body, &mut i)?);
            let first = *body.get(i)?;
            let status = if first & 0x80 != 0 {
                i += 1;
                first
            } else {
                running
            };
            order += 1;
            match status {
                0xFF => {
                    let kind = *body.get(i)?;
                    i += 1;
                    let n = vlq(body, &mut i)? as usize;
                    let data = body.get(i..i + n)?;
                    i += n;
                    match kind {
                        0x51 if n == 3 => events.push((
                            tick,
                            order,
                            Event::Tempo(u32::from_be_bytes([0, data[0], data[1], data[2]])),
                        )),
                        0x2F => break,
                        _ => {}
                    }
                }
                0xF0 | 0xF7 => {
                    let n = vlq(body, &mut i)? as usize;
                    let data = body.get(i..i + n)?;
                    i += n;
                    if status == 0xF0 {
                        let mut message = Vec::with_capacity(n + 1);
                        message.push(0xF0);
                        message.extend_from_slice(data);
                        events.push((tick, order, Event::Sysex(message)));
                    }
                }
                0x80..=0xEF => {
                    running = status;
                    let data_bytes = if matches!(status & 0xF0, 0xC0 | 0xD0) {
                        1
                    } else {
                        2
                    };
                    let a = *body.get(i)?;
                    let c = if data_bytes == 2 {
                        *body.get(i + 1)?
                    } else {
                        0
                    };
                    i += data_bytes;
                    events.push((
                        tick,
                        order,
                        Event::Short(u32::from(status) | u32::from(a) << 8 | u32::from(c) << 16),
                    ));
                }
                _ => return None,
            }
        }
        end = end.max(tick);
    }
    // Merge: by tick, then by file order.
    events.sort_by_key(|(tick, order, _)| (*tick, *order));
    Some(Song {
        division,
        events: events.into_iter().map(|(t, _, e)| (t, e)).collect(),
        end,
    })
}
impl Song {
    /// Microseconds from the start to `tick`, through the tempo changes.
    pub fn micros(&self, tick: u64) -> u64 {
        let mut tempo = 500_000u64;
        let (mut at, mut us) = (0u64, 0u64);
        for (t, e) in &self.events {
            if *t >= tick {
                break;
            }
            if let Event::Tempo(new) = e {
                us += (t - at) * tempo / u64::from(self.division);
                at = *t;
                tempo = u64::from(*new);
            }
        }
        us + (tick - at) * tempo / u64::from(self.division)
    }
    /// The song's length in milliseconds.
    pub fn length_ms(&self) -> u64 {
        self.micros(self.end) / 1000
    }
    /// The tick `micros` microseconds into the song (the inverse of
    /// `micros`, rounded down).
    pub fn tick_at(&self, micros: u64) -> u64 {
        let mut tempo = 500_000u64;
        let (mut at, mut us) = (0u64, 0u64);
        let division = u64::from(self.division);
        for (t, e) in &self.events {
            if let Event::Tempo(new) = e {
                let span = (t - at) * tempo / division;
                if us + span > micros {
                    break;
                }
                us += span;
                at = *t;
                tempo = u64::from(*new).max(1);
            }
        }
        (at + (micros - us) * division / tempo).min(self.end)
    }
    /// The events from `start` to `end` (ticks) as (delta, event) for the
    /// synthesizer: what precedes `start` except note-ons comes first at
    /// delta 0 (programs, controllers, tempo), so the song resumes in the
    /// state it would have reached.
    pub fn from(&self, start: u64, end: u64) -> Vec<(u32, Event)> {
        let mut out = Vec::new();
        let mut last = start;
        for (t, e) in &self.events {
            if *t >= end {
                break;
            }
            if *t < start {
                let note_on =
                    matches!(e, Event::Short(w) if w & 0xF0 == 0x90 && (w >> 16) & 0x7F != 0);
                if !note_on {
                    out.push((0, e.clone()));
                }
                continue;
            }
            out.push(((t - last) as u32, e.clone()));
            last = *t;
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    /// The MIDI-A probe's song: C6 on a square lead for two beats.
    const SONG: [u8; 49] = [
        b'M', b'T', b'h', b'd', 0, 0, 0, 6, 0, 0, 0, 1, 0, 96, b'M', b'T', b'r', b'k', 0, 0, 0, 27,
        0, 0xFF, 0x51, 3, 0x07, 0xA1, 0x20, 0, 0xC0, 80, 0, 0xB0, 7, 100, 0, 0x90, 84, 110, 0x81,
        0x40, 0x80, 84, 0, 0, 0xFF, 0x2F, 0,
    ];
    #[test]
    fn a_format_zero_song() {
        let song = parse(&SONG).unwrap();
        assert_eq!(song.division, 96);
        assert_eq!(song.end, 192);
        assert_eq!(song.length_ms(), 1000);
        assert_eq!(
            song.events,
            [
                (0, Event::Tempo(500_000)),
                (0, Event::Short(0x00_50C0)),
                (0, Event::Short(0x64_07B0)),
                (0, Event::Short(0x6E_5490)),
                (192, Event::Short(0x00_5480)),
            ]
        );
        // Seeking: half a second is beat one (tick 96); playing from there
        // keeps the program, tempo and volume but not the note-on.
        assert_eq!(song.tick_at(500_000), 96);
        assert_eq!(song.tick_at(10_000_000), 192);
        assert_eq!(
            song.from(96, 193),
            [
                (0, Event::Tempo(500_000)),
                (0, Event::Short(0x00_50C0)),
                (0, Event::Short(0x64_07B0)),
                (96, Event::Short(0x00_5480)),
            ]
        );
        // RIFF RMID wrapping.
        let mut rmid = b"RIFF\0\0\0\0RMIDdata".to_vec();
        rmid.extend((SONG.len() as u32).to_le_bytes());
        rmid.extend(SONG);
        assert_eq!(parse(&rmid), Some(song));
    }
    #[test]
    fn tracks_merge_and_tempo_changes() {
        // Format 1: a tempo track (120 bpm, then 60 bpm at tick 96) and a
        // note track with running status.
        let mut b = b"MThd\0\0\0\x06\0\x01\0\x02\0\x60".to_vec();
        let tempo = [
            0, 0xFF, 0x51, 3, 0x07, 0xA1, 0x20, 0x60, 0xFF, 0x51, 3, 0x0F, 0x42, 0x40, 0, 0xFF,
            0x2F, 0,
        ];
        b.extend(b"MTrk");
        b.extend((tempo.len() as u32).to_be_bytes());
        b.extend(tempo);
        let notes = [0, 0x91, 60, 100, 0x81, 0x40, 60, 0, 0, 0xFF, 0x2F, 0];
        b.extend(b"MTrk");
        b.extend((notes.len() as u32).to_be_bytes());
        b.extend(notes);
        let song = parse(&b).unwrap();
        assert_eq!(song.end, 192);
        // 96 ticks at half a second per beat, 96 at a second per beat.
        assert_eq!(song.length_ms(), 1500);
        assert_eq!(song.events.last(), Some(&(192, Event::Short(0x00_3C91))));
        // Truncated or foreign data is refused.
        assert!(parse(&b[..b.len() - 3]).is_none());
        assert!(parse(b"MThd\0\0\0\x06\0\x02\0\x01\0\x60").is_none());
        assert!(parse(b"RIFF").is_none());
    }
}
