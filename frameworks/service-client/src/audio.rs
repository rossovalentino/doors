//! door.audio client (P15): the sound service's status and master output,
//! and playback streams fed with PCM sections.
use crate::{Error, Result, rpc::Connection};
use door_abi::{Handle, Transfer, rights};
use door_service_protocol::audio::{self, Format, Output, StreamStatus, op};

pub struct Audio {
    connection: Connection,
}
impl Audio {
    pub fn connect(registry: Handle) -> Result<Self> {
        Ok(Self {
            connection: crate::connect(registry, door_abi::platform::manifest::AUDIO)?,
        })
    }
    /// Connect through a port already resolved (`Init::begin_resolve`); the
    /// caller keeps and closes `port`.
    pub fn connect_port(port: Handle) -> Result<Self> {
        Ok(Self {
            connection: Connection::connect(port, door_abi::platform::manifest::AUDIO)?,
        })
    }
    pub fn status(&mut self) -> Result<audio::Status> {
        let reply = self.connection.call(op::STATUS, &[], &[])?;
        audio::Status::decode(reply.data()?).ok_or(Error::Malformed)
    }
    /// The display service and Settings only.
    pub fn set_output(&mut self, output: Output) -> Result<()> {
        self.connection
            .call(op::SET_OUTPUT, &output.encode(), &[])?
            .empty()
    }
    /// Settings' Test button: a one-second 440 Hz tone; its stream id.
    pub fn test_tone(&mut self) -> Result<u32> {
        Ok(self.connection.call(op::TEST_TONE, &[], &[])?.scalar()? as u32)
    }
    /// Play a system sound (`audio::sound` event) from the sound scheme.
    pub fn play_sound(&mut self, event: u32) -> Result<u32> {
        Ok(self
            .connection
            .call(op::PLAY_SOUND, &event.to_le_bytes(), &[])?
            .scalar()? as u32)
    }
    pub fn open(&mut self, format: Format) -> Result<u32> {
        Ok(self
            .connection
            .call(op::OPEN, &format.encode(), &[])?
            .scalar()? as u32)
    }
    /// Queue the first `bytes` of `section` (whole frames, at most
    /// `audio::MAX_SECTION`; see `door_workspace::section_from`); the caller
    /// closes its handle afterwards. `Err(Service(WouldBlock))` while two
    /// sections of the stream are still waiting.
    pub fn queue(&mut self, id: u32, section: Handle, bytes: usize) -> Result<u64> {
        let mut body = [0u8; 8];
        body[..4].copy_from_slice(&id.to_le_bytes());
        body[4..].copy_from_slice(&(bytes as u32).to_le_bytes());
        // The service maps the section read-only, which the kernel leases
        // like a mapping: READ and MAP on the grant.
        let transfer = Transfer {
            handle: section,
            rights: rights::QUERY | rights::READ | rights::MAP | rights::TRANSFER,
            mode: 0,
        };
        self.connection
            .call(op::QUEUE, &body, &[transfer])?
            .scalar()
    }
    pub fn stream(&mut self, id: u32) -> Result<StreamStatus> {
        let reply = self.connection.call(op::STREAM, &id.to_le_bytes(), &[])?;
        StreamStatus::decode(reply.data()?).ok_or(Error::Malformed)
    }
    pub fn pause(&mut self, id: u32, paused: bool) -> Result<()> {
        let mut body = [0u8; 5];
        body[..4].copy_from_slice(&id.to_le_bytes());
        body[4] = u8::from(paused);
        self.connection.call(op::PAUSE, &body, &[])?.empty()
    }
    pub fn stream_volume(&mut self, id: u32, volume: u8) -> Result<()> {
        let mut body = [0u8; 5];
        body[..4].copy_from_slice(&id.to_le_bytes());
        body[4] = volume.min(100);
        self.connection.call(op::STREAM_VOLUME, &body, &[])?.empty()
    }
    pub fn close(&mut self, id: u32) -> Result<()> {
        self.connection
            .call(op::CLOSE, &id.to_le_bytes(), &[])?
            .empty()
    }
    /// Open a capture stream into the ring in `section` (`audio::capture`
    /// layout, its header prepared); the caller keeps its own handle and
    /// mapping. `Err(Service(Denied))` when the microphone policy refuses.
    /// `name`: what the Microphone page lists (at most `audio::NAME_BYTES`).
    pub fn capture_open(&mut self, section: Handle, name: &str) -> Result<u32> {
        // The service maps the ring read-write and writes into it.
        let transfer = Transfer {
            handle: section,
            rights: rights::QUERY | rights::READ | rights::WRITE | rights::MAP | rights::TRANSFER,
            mode: 0,
        };
        Ok(self
            .connection
            .call(op::CAPTURE_OPEN, name.as_bytes(), &[transfer])?
            .scalar()? as u32)
    }
    pub fn capture_close(&mut self, id: u32) -> Result<()> {
        self.connection
            .call(op::CAPTURE_CLOSE, &id.to_le_bytes(), &[])?
            .empty()
    }
    /// The display service and Settings only.
    /// The microphone level (`op::SET_INPUT`).
    pub fn set_input(&mut self, input: audio::Input) -> Result<()> {
        self.connection
            .call(op::SET_INPUT, &input.encode(), &[])?
            .empty()
    }
    pub fn capture_policy(&mut self, policy: u32) -> Result<()> {
        self.connection
            .call(op::CAPTURE_POLICY, &policy.to_le_bytes(), &[])?
            .empty()
    }
    pub fn capture_status(&mut self) -> Result<audio::CaptureStatus> {
        let reply = self.connection.call(op::CAPTURE_STATUS, &[], &[])?;
        audio::CaptureStatus::decode(reply.data()?).ok_or(Error::Malformed)
    }
}
