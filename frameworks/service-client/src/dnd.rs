//! door.dnd v1 client (allocation-free). Request CAP_DRAG_DROP (and
//! CAP_CLIPBOARD, whose staging a drag shares) with
//! `WindowClient::connect_with`.
use crate::window::WindowClient;
use crate::{Error, Result};
use door_abi::Handle;
use door_service_protocol::{Code, clipboard as cb, dnd};
fn body(window: u64, words: &[u32], out: &mut [u8]) -> usize {
    out[..8].copy_from_slice(&window.to_le_bytes());
    for (i, v) in words.iter().enumerate() {
        out[8 + 4 * i..12 + 4 * i].copy_from_slice(&v.to_le_bytes());
    }
    8 + 4 * words.len()
}
impl WindowClient {
    pub fn supports_drag_drop(&self) -> bool {
        self.capabilities & dnd::CAP_DRAG_DROP != 0
    }
    fn dnd_call(&mut self, op: u32, b: &[u8]) -> Result<crate::rpc::Reply> {
        if !self.supports_drag_drop() {
            return Err(Error::Service(Code::Unsupported));
        }
        self.connection.call(op, b, &[])
    }
    /// RegisterDragDrop: `window` accepts `formats` (`cb::mask` bits) with
    /// `effects`, usually `default`.
    pub fn register_drop_target(
        &mut self,
        window: u64,
        formats: u32,
        effects: u32,
        default: u32,
    ) -> Result<()> {
        let mut b = [0u8; 20];
        let n = body(window, &[formats, effects, default], &mut b);
        self.dnd_call(dnd::REGISTER, &b[..n])?.empty()
    }
    pub fn revoke_drop_target(&mut self, window: u64) -> Result<()> {
        self.dnd_call(dnd::REVOKE, &window.to_le_bytes())?.empty()
    }
    /// DoDragDrop from `window` while the primary button is held: stage the
    /// offer and start. Returns the drag serial; FINISHED reports the end.
    pub fn start_drag(
        &mut self,
        window: u64,
        effects: u32,
        formats: &[(u32, &[u8])],
    ) -> Result<u32> {
        self.start_drag_with(window, effects, formats, &[])
    }
    /// As `start_drag`, with large formats in sections `(format, section, bytes)`.
    pub fn start_drag_with(
        &mut self,
        window: u64,
        effects: u32,
        formats: &[(u32, &[u8])],
        sections: &[(u32, Handle, u32)],
    ) -> Result<u32> {
        let mut b = [0u8; 12];
        let n = body(window, &[effects], &mut b);
        self.dnd_call(dnd::BEGIN, &b[..n])?.empty()?;
        let staged = self.stage_formats(formats).and_then(|_| {
            sections.iter().try_for_each(|(format, section, bytes)| {
                self.stage_section(*format, *section, *bytes)
            })
        });
        if let Err(e) = staged {
            let _ = self.connection.call(cb::ABORT, &[], &[]);
            return Err(e);
        }
        let r = self.connection.call(dnd::START, &[], &[])?;
        let data = r.data()?;
        if data.len() != 4 {
            return Err(Error::Malformed);
        }
        Ok(u32::from_le_bytes(data.try_into().unwrap()))
    }
    /// Answer ENTER/OVER: what this target would do at the pointer.
    pub fn drag_status(
        &mut self,
        serial: u32,
        window: u64,
        effects: u32,
        default: u32,
    ) -> Result<()> {
        self.drag_status_described(serial, window, effects, default, "")
    }
    /// `drag_status` with the drop description insert (DROPDESCRIPTION
    /// szInsert): the name of what is under the pointer, shown as "Copy to
    /// Documents" / "Move to Documents". Empty: the bare effect label.
    pub fn drag_status_described(
        &mut self,
        serial: u32,
        window: u64,
        effects: u32,
        default: u32,
        insert: &str,
    ) -> Result<()> {
        if !dnd::valid_description(insert) {
            return Err(Error::Service(Code::InvalidRequest));
        }
        let mut b = [0u8; 20 + dnd::MAX_DESCRIPTION];
        b[..4].copy_from_slice(&serial.to_le_bytes());
        b[4..12].copy_from_slice(&window.to_le_bytes());
        b[12..16].copy_from_slice(&effects.to_le_bytes());
        b[16..20].copy_from_slice(&default.to_le_bytes());
        b[20..20 + insert.len()].copy_from_slice(insert.as_bytes());
        self.dnd_call(dnd::STATUS, &b[..20 + insert.len()])?.empty()
    }
    /// After DROP: one chunk of an accepted format (0 bytes at the end).
    pub fn drop_read(
        &mut self,
        serial: u32,
        format: u32,
        offset: u32,
        out: &mut [u8],
    ) -> Result<usize> {
        let mut b = [0u8; 12];
        b[..4].copy_from_slice(&serial.to_le_bytes());
        b[4..8].copy_from_slice(&format.to_le_bytes());
        b[8..12].copy_from_slice(&offset.to_le_bytes());
        let r = self.dnd_call(dnd::READ, &b)?;
        let data = r.data()?;
        if data.len() > out.len() || data.len() > cb::READ_BYTES {
            return Err(Error::Malformed);
        }
        out[..data.len()].copy_from_slice(data);
        Ok(data.len())
    }
    /// The drop is handled: the performed effect (`dnd::NONE` on failure).
    pub fn drop_finish(&mut self, serial: u32, effect: u32) -> Result<()> {
        let mut b = [0u8; 8];
        b[..4].copy_from_slice(&serial.to_le_bytes());
        b[4..].copy_from_slice(&effect.to_le_bytes());
        self.dnd_call(dnd::FINISH, &b)?.empty()
    }
    pub fn cancel_drag(&mut self, serial: u32) -> Result<()> {
        self.dnd_call(dnd::CANCEL, &serial.to_le_bytes())?.empty()
    }
}
