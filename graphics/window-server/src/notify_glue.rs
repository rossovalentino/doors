//! Shell_NotifyIcon on the desktop: Windows programs' requests, the
//! hidden-icons flyout's clicks back to them (`door_window::notify_icons`).
use super::*;
use door_abi::{HandleKind, Transfer, rights};
use door_system_services::server;
use door_window::notify_icons as ni;
use door_window::tray;

impl Desktop {
    /// `window::NOTIFY_ICON` from a Windows program.
    pub(super) fn notify_icon_request(
        &mut self,
        owner: g::Owner,
        manifest: u64,
        b: &[u8],
        t: &[Transfer],
    ) -> Result<Response, Code> {
        use door_abi::platform::manifest::{WINDOWS, WINDOWS_APPLICATION, WINDOWS_GUI};
        use w::notify;
        // Console programs with a window (a hidden message window, as most
        // tray utilities started from a console have) may add icons too.
        if !matches!(manifest, WINDOWS | WINDOWS_APPLICATION | WINDOWS_GUI) {
            return Err(Code::Denied);
        }
        let request = ni::decode(b)?;
        if request.op == notify::RECT {
            if !t.is_empty() {
                return Err(Code::InvalidRequest);
            }
            let at = self
                .tray
                .notify
                .find(owner, request.window, request.id)
                .ok_or(Code::NotFound)?;
            let r = match self.hidden_flyout() {
                Some((flyout, count)) if at < count => ni::cell(flyout, at),
                _ => ni::chevron(self.taskbar_layout(), self.tray.recording()),
            };
            let mut out = [0u8; 16];
            out[..4].copy_from_slice(&r.x().to_le_bytes());
            out[4..8].copy_from_slice(&r.y().to_le_bytes());
            out[8..12].copy_from_slice(&r.width().to_le_bytes());
            out[12..].copy_from_slice(&r.height().to_le_bytes());
            return Ok(Response::bytes(&out));
        }
        let with_icon =
            request.flags & notify::ICON != 0 && matches!(request.op, notify::ADD | notify::MODIFY);
        let pixels = match (with_icon, t) {
            (true, [section]) => {
                let info = door_sys::inspect(section.handle).map_err(server::status)?;
                if info[0] != HandleKind::Section as u64 || info[1] & rights::WRITE != 0 {
                    return Err(Code::Denied);
                }
                let mut bytes = alloc::vec![0; notify::ICON_BYTES];
                for (i, chunk) in bytes
                    .chunks_mut(door_abi::graphics::SECTION_COPY_BYTES)
                    .enumerate()
                {
                    let at = (i * door_abi::graphics::SECTION_COPY_BYTES) as u64;
                    if door_sys::section_snapshot(section.handle, at, chunk)
                        .map_err(server::status)?
                        != chunk.len()
                    {
                        return Err(Code::InvalidRequest);
                    }
                }
                // Premultiplied: no channel above its alpha.
                for p in bytes.chunks_exact_mut(4) {
                    let a = p[3];
                    for c in &mut p[..3] {
                        *c = (*c).min(a);
                    }
                }
                Some(bytes)
            }
            (false, []) => None,
            _ => return Err(Code::InvalidRequest),
        };
        self.tray.notify.apply(owner, &request, pixels)?;
        // An open hidden-icons flyout follows the icons.
        if matches!(self.tray.open, Some(tray::Flyout::Hidden(_))) {
            self.tray.open = (!self.tray.notify.is_empty())
                .then(|| tray::Flyout::Hidden(self.tray.notify.len().min(255) as u8));
        }
        let _ = door_sys::write(
            format!(
                "DOOR_P16 NOTIFY_ICON op={} id={} icons={}\n",
                request.op,
                request.id,
                self.tray.notify.len()
            )
            .as_bytes(),
        );
        self.engine.invalidate();
        Ok(Response::empty())
    }
    /// The open hidden-icons flyout's rectangle.
    fn hidden_flyout(&self) -> Option<(door_graphics_core::Rect, usize)> {
        let Some(tray::Flyout::Hidden(n)) = self.tray.open else {
            return None;
        };
        let count = usize::from(n).min(self.tray.notify.len());
        Some((
            ni::flyout(self.taskbar_layout(), self.tray.recording(), count),
            count,
        ))
    }
    /// A click on icon `index` of the flyout: its owner hears it, and the
    /// flyout closes as Windows' does.
    pub(super) fn notify_icon_click(&mut self, index: usize, secondary: bool) {
        let Some((flyout, count)) = self.hidden_flyout() else {
            return;
        };
        if index >= count {
            return;
        }
        let kind = if secondary {
            ni::RIGHT
        } else {
            self.tray
                .notify
                .primary(index, door_sys::time().unwrap_or(0))
        };
        let sent = self.tray.notify.click(index, kind, ni::cell(flyout, index));
        let _ = door_sys::write(
            format!("DOOR_P16 NOTIFY_ICON click index={index} kind={kind} sent={sent}\n")
                .as_bytes(),
        );
        // A double click keeps nothing open; the first click already closed.
        self.tray.open = None;
        self.engine.invalidate();
    }
    /// A secondary click over the hidden-icons flyout: true when it hit an
    /// icon (the owner shows its own menu).
    pub(super) fn notify_icon_secondary(&mut self, x: i32, y: i32) -> bool {
        let Some((flyout, count)) = self.hidden_flyout() else {
            return false;
        };
        match ni::hit(flyout, count, x, y) {
            Some(index) => {
                self.notify_icon_click(index, true);
                true
            }
            None => false,
        }
    }
}
