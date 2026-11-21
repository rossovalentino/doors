//! Audio playback for Preview (P15): the open file's decoded PCM, narrowed
//! to 16-bit mono or stereo, streamed to door.audio in sections of about
//! two seconds while no more than four seconds wait in the mixer. Seeking
//! restarts the stream at the new frame; pausing pauses the stream.
extern crate alloc;
use alloc::vec::Vec;
use door_service_client::audio::Audio;
use door_service_protocol::audio::Format;
use door_workspace::audio_codec::Pcm;

/// Seconds of audio per queued section, and the most kept waiting.
const SECTION_SECONDS: usize = 2;
const AHEAD_SECONDS: u64 = 4;

pub struct Player {
    link: Audio,
    stream: u32,
    /// The tab playing (its id).
    pub tab: u64,
    /// The whole file as interleaved 16-bit little-endian PCM.
    pcm: Vec<u8>,
    rate: u32,
    channels: usize,
    /// First frame of the current stream, frames queued past it, and frames
    /// the mixer reports played.
    start: usize,
    queued: usize,
    played: u64,
    pub paused: bool,
    pub finished: bool,
}
/// 16-bit PCM bytes of `pcm`, keeping at most two channels.
pub fn narrow(pcm: &Pcm) -> (Vec<u8>, usize) {
    let all = usize::from(pcm.channels.max(1));
    let keep = all.min(2);
    let shift = i32::from(pcm.bits) - 16;
    let mut out = Vec::with_capacity(pcm.frames() * keep * 2);
    for frame in pcm.samples.chunks_exact(all) {
        for &s in &frame[..keep] {
            let v = if shift >= 0 { s >> shift } else { s << -shift };
            out.extend_from_slice(&(v.clamp(-32768, 32767) as i16).to_le_bytes());
        }
    }
    (out, keep)
}
impl Player {
    pub fn start(registry: u64, tab: u64, pcm: &Pcm, from: usize) -> Result<Self, &'static str> {
        if registry == 0 {
            return Err("Playback isn't available in this session.");
        }
        let mut link = Audio::connect(registry).map_err(|_| "Sound service isn't available.")?;
        let status = link
            .status()
            .map_err(|_| "Sound service isn't available.")?;
        if !status.present {
            return Err("No audio output device is available.");
        }
        let (bytes, channels) = narrow(pcm);
        let format = Format {
            rate: pcm.rate,
            channels: channels as u8,
            bits: 16,
        };
        let stream = link
            .open(format)
            .map_err(|_| "The sound service refused this audio format.")?;
        let mut player = Self {
            link,
            stream,
            tab,
            pcm: bytes,
            rate: pcm.rate,
            channels,
            start: from.min(pcm.frames()),
            queued: 0,
            played: 0,
            paused: false,
            finished: false,
        };
        player.feed()?;
        door_rt::diagnostic(format_args!(
            "DOOR_P15 PREVIEW PLAY rate={} channels={} frames={} from={}\n",
            pcm.rate,
            channels,
            pcm.frames(),
            player.start
        ));
        Ok(player)
    }
    fn frames(&self) -> usize {
        self.pcm.len() / (2 * self.channels)
    }
    /// Queue more sections while less than `AHEAD_SECONDS` wait.
    fn feed(&mut self) -> Result<(), &'static str> {
        let rate = self.rate as usize;
        while self.start + self.queued < self.frames()
            && (self.queued as u64).saturating_sub(self.played) < AHEAD_SECONDS * rate as u64
        {
            let from = self.start + self.queued;
            let n = (self.frames() - from).min(SECTION_SECONDS * rate);
            let frame = 2 * self.channels;
            let bytes = &self.pcm[from * frame..(from + n) * frame];
            let section =
                door_workspace::section_from(bytes).map_err(|_| "Out of memory for playback.")?;
            let queued = self.link.queue(self.stream, section, bytes.len());
            let _ = door_sys::close(section);
            match queued {
                Ok(_) => self.queued += n,
                Err(door_service_client::Error::Service(
                    door_service_protocol::Code::WouldBlock,
                )) => break,
                Err(_) => return Err("Playback stopped: the sound service failed."),
            }
        }
        Ok(())
    }
    /// Poll the stream and keep it fed; true when the position changed.
    pub fn tick(&mut self) -> Result<bool, &'static str> {
        if self.finished {
            return Ok(false);
        }
        let status = self
            .link
            .stream(self.stream)
            .map_err(|_| "Playback stopped: the sound service failed.")?;
        let moved = status.played != self.played;
        self.played = status.played;
        self.feed()?;
        if self.start + self.queued >= self.frames() && status.drained {
            self.finished = true;
            let _ = self.link.close(self.stream);
            return Ok(true);
        }
        Ok(moved)
    }
    /// Current frame.
    pub fn position(&self) -> usize {
        (self.start + self.played as usize).min(self.frames())
    }
    pub fn toggle_pause(&mut self) -> bool {
        if self.finished {
            return false;
        }
        let next = !self.paused;
        if self.link.pause(self.stream, next).is_ok() {
            self.paused = next;
        }
        true
    }
    /// Restart at `frame`, keeping the pause state.
    pub fn seek(&mut self, frame: usize) -> Result<(), &'static str> {
        let _ = self.link.close(self.stream);
        let format = Format {
            rate: self.rate,
            channels: self.channels as u8,
            bits: 16,
        };
        self.stream = self
            .link
            .open(format)
            .map_err(|_| "Playback stopped: the sound service failed.")?;
        self.start = frame.min(self.frames());
        self.queued = 0;
        self.played = 0;
        self.finished = false;
        if self.paused {
            let _ = self.link.pause(self.stream, true);
        }
        self.feed()
    }
}
impl Drop for Player {
    fn drop(&mut self) {
        if !self.finished {
            let _ = self.link.close(self.stream);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn narrowing_keeps_two_channels_at_16_bits() {
        let pcm = Pcm {
            rate: 8000,
            channels: 3,
            bits: 24,
            samples: alloc::vec![256, -256, 7, 0x7FFF00, 0, 1],
        };
        let (bytes, channels) = narrow(&pcm);
        assert_eq!(channels, 2);
        assert_eq!(bytes, [1, 0, 0xFF, 0xFF, 0xFF, 0x7F, 0, 0]);
    }
}
