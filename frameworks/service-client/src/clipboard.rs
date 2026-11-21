//! door.clipboard v1 client (allocation-free). Request CAP_CLIPBOARD with
//! `WindowClient::connect_with(registry, clipboard::CAP_CLIPBOARD)`.
use crate::window::WindowClient;
use crate::{Error, Result};
use door_abi::{Handle, Transfer, rights};
use door_service_protocol::{
    Code,
    clipboard::{self as cb, Info},
    dnd,
    window::Event,
};
/// Clipboard and drag/drop events for a client that negotiated them.
pub(crate) fn decode_event(b: &[u8], capabilities: u32) -> Option<Event> {
    if capabilities & cb::CAP_CLIPBOARD != 0
        && let Ok(e) = cb::decode(b)
    {
        return Some(e);
    }
    if capabilities & dnd::CAP_DRAG_DROP != 0 {
        return dnd::decode(b).ok();
    }
    None
}
fn words(values: &[u32], out: &mut [u8]) -> usize {
    for (i, v) in values.iter().enumerate() {
        out[4 * i..4 * i + 4].copy_from_slice(&v.to_le_bytes());
    }
    4 * values.len()
}
impl WindowClient {
    pub fn supports_clipboard(&self) -> bool {
        self.capabilities & cb::CAP_CLIPBOARD != 0
    }
    fn clipboard_call(&mut self, op: u32, body: &[u8]) -> Result<crate::rpc::Reply> {
        if !self.supports_clipboard() {
            return Err(Error::Service(Code::Unsupported));
        }
        self.connection.call(op, body, &[])
    }
    /// Declare and write staged formats (shared by copy and drag).
    pub(crate) fn stage_formats(&mut self, formats: &[(u32, &[u8])]) -> Result<()> {
        for (format, bytes) in formats {
            let mut b = [0u8; 8 + cb::WRITE_BYTES];
            let n = words(&[*format, bytes.len() as u32], &mut b);
            self.connection.call(cb::FORMAT, &b[..n], &[])?.empty()?;
            for (i, chunk) in bytes.chunks(cb::WRITE_BYTES).enumerate() {
                words(&[*format, (i * cb::WRITE_BYTES) as u32], &mut b);
                b[8..8 + chunk.len()].copy_from_slice(chunk);
                self.connection
                    .call(cb::WRITE, &b[..8 + chunk.len()], &[])?
                    .empty()?;
            }
        }
        Ok(())
    }
    /// Stage one large format from a section the caller filled (`bytes` from
    /// offset 0); the server snapshots it once.
    pub(crate) fn stage_section(&mut self, format: u32, section: Handle, bytes: u32) -> Result<()> {
        let mut b = [0u8; 12];
        let n = words(&[format, bytes], &mut b);
        self.connection.call(cb::FORMAT, &b[..n], &[])?.empty()?;
        words(&[format, 0, bytes], &mut b);
        self.connection
            .call(
                cb::WRITE_SECTION,
                &b,
                // The server snapshots the section, which leases it like a
                // mapping: the kernel requires READ and MAP on the grant.
                &[Transfer {
                    handle: section,
                    rights: rights::QUERY | rights::READ | rights::MAP | rights::TRANSFER,
                    mode: 0,
                }],
            )?
            .empty()
    }
    fn clipboard_commit(&mut self) -> Result<u32> {
        let r = self.connection.call(cb::COMMIT, &[], &[])?;
        let b = r.data()?;
        if b.len() != 4 {
            return Err(Error::Malformed);
        }
        Ok(u32::from_le_bytes(b.try_into().unwrap()))
    }
    /// OpenClipboard + EmptyClipboard + SetClipboardData + CloseClipboard:
    /// replace the clipboard with `formats` for `window` (none: empty it).
    /// Returns the new sequence number.
    pub fn clipboard_set(&mut self, window: u64, formats: &[(u32, &[u8])]) -> Result<u32> {
        self.clipboard_set_with(window, formats, &[])
    }
    /// As `clipboard_set`, with large formats in sections:
    /// `(format, section, bytes)`.
    pub fn clipboard_set_with(
        &mut self,
        window: u64,
        formats: &[(u32, &[u8])],
        sections: &[(u32, Handle, u32)],
    ) -> Result<u32> {
        self.clipboard_call(cb::BEGIN, &window.to_le_bytes())?
            .empty()?;
        let staged = self.stage_formats(formats).and_then(|_| {
            sections.iter().try_for_each(|(format, section, bytes)| {
                self.stage_section(*format, *section, *bytes)
            })
        });
        if let Err(e) = staged {
            let _ = self.connection.call(cb::ABORT, &[], &[]);
            return Err(e);
        }
        self.clipboard_commit()
    }
    /// Sequence, available formats, owner state and sizes.
    pub fn clipboard_info(&mut self) -> Result<Info> {
        let r = self.clipboard_call(cb::INFO, &[])?;
        Info::decode(r.data()?).map_err(|_| Error::Malformed)
    }
    /// One READ chunk of `format` at `offset` (0 bytes at the end).
    /// `Error::Service(Code::Changed)` when the clipboard changed since
    /// `sequence`; `Denied` unless this client owns the focused window.
    pub fn clipboard_read(
        &mut self,
        sequence: u32,
        format: u32,
        offset: u32,
        out: &mut [u8],
    ) -> Result<usize> {
        let mut b = [0u8; 12];
        let n = words(&[sequence, format, offset], &mut b);
        let r = self.clipboard_call(cb::READ, &b[..n])?;
        let data = r.data()?;
        if data.len() > out.len() || data.len() > cb::READ_BYTES {
            return Err(Error::Malformed);
        }
        out[..data.len()].copy_from_slice(data);
        Ok(data.len())
    }
    /// Read a whole format into `out` (use `Info::size` to size it).
    pub fn clipboard_read_into(
        &mut self,
        info: &Info,
        format: u32,
        out: &mut [u8],
    ) -> Result<usize> {
        let size = info.size(format) as usize;
        if size == 0 {
            return Err(Error::Service(Code::NotFound));
        }
        if size > out.len() {
            return Err(Error::Service(Code::Resources));
        }
        let mut done = 0;
        while done < size {
            let end = (done + cb::READ_BYTES).min(size);
            let n = self.clipboard_read(info.sequence, format, done as u32, &mut out[done..end])?;
            if n == 0 {
                return Err(Error::Malformed);
            }
            done += n;
        }
        Ok(size)
    }
    /// Settings > System > Clipboard bits (`cb::SETTING_*`).
    pub fn clipboard_settings(&mut self) -> Result<u32> {
        let r = self.clipboard_call(cb::SETTINGS_GET, &[])?;
        let b = r.data()?;
        if b.len() != 4 {
            return Err(Error::Malformed);
        }
        Ok(u32::from_le_bytes(b.try_into().unwrap()))
    }
    /// Settings app only. Turning history off clears it, pinned included.
    pub fn set_clipboard_settings(&mut self, settings: u32) -> Result<()> {
        self.clipboard_call(cb::SETTINGS_SET, &settings.to_le_bytes())?
            .empty()
    }
    /// Settings app only: "Clear clipboard data".
    pub fn clear_clipboard_data(&mut self) -> Result<()> {
        self.clipboard_call(cb::CLEAR_DATA, &[])?.empty()
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn extension_events_reach_only_clients_that_negotiated_them() {
        let changed = Event {
            kind: cb::CHANGED,
            a: 3,
            ..Event::default()
        }
        .encode();
        let drop = Event {
            kind: dnd::DROP,
            value: 2,
            ..Event::default()
        }
        .encode();
        assert_eq!(decode_event(&changed, 0), None);
        assert_eq!(
            decode_event(&changed, cb::CAP_CLIPBOARD).map(|e| e.a),
            Some(3)
        );
        assert_eq!(decode_event(&drop, cb::CAP_CLIPBOARD), None);
        assert_eq!(
            decode_event(&drop, dnd::CAP_DRAG_DROP).map(|e| e.value),
            Some(2)
        );
        let input = Event {
            kind: door_service_protocol::window::INPUT,
            ..Event::default()
        }
        .encode();
        assert_eq!(
            decode_event(&input, cb::CAP_CLIPBOARD | dnd::CAP_DRAG_DROP),
            None,
            "window events stay with window::Event::decode"
        );
    }
}
