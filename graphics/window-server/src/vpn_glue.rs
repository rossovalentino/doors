//! Quick settings VPN (P15): the session user's connections from
//! door.network (`net::vpn`), refreshed every 3 s and every second while
//! Quick settings is open, and the page's Connect/Disconnect button.
//! door.network trusts this server to name the user of the session it
//! shows; it never lets it add or remove connections (Settings does).
use super::*;
use door_service_protocol::net::vpn::State;
use door_window::tray;

impl Desktop {
    pub(super) fn vpn_tick(&mut self, now: u64) {
        if now < self.tray.vpn_at {
            return;
        }
        let open = self.tray.open == Some(tray::Flyout::Devices);
        self.tray.vpn_at = now.saturating_add(if open { 100 } else { 300 });
        let uid = self.session.map(|(_, _, uid)| uid);
        let Some(link) = self.tray.network_link.as_mut() else {
            return;
        };
        let mut vpns = alloc::vec::Vec::new();
        if let Some(uid) = uid {
            for index in 0..=u8::MAX {
                match link.vpn_entry(uid, index) {
                    Ok(Some(e)) => vpns.push(tray::VpnEntry {
                        name: e.name().into(),
                        connected: e.state == State::Connected,
                        connecting: e.state == State::Connecting,
                    }),
                    Ok(None) => break,
                    // network_tick drops and re-resolves a failed link.
                    Err(_) => return,
                }
            }
        }
        // The connection the user picked stays first (expanded).
        if let Some(first) = self.tray.vpns.first().map(|v| v.name.clone())
            && let Some(at) = vpns.iter().position(|v| v.name == first)
        {
            vpns[..=at].rotate_right(1);
        }
        if vpns != self.tray.vpns {
            let changed_state = vpns.iter().map(|v| (v.connected, v.connecting)).ne(self
                .tray
                .vpns
                .iter()
                .map(|v| (v.connected, v.connecting)));
            self.tray.vpns = vpns;
            self.bar_dirty = true;
            if open {
                self.engine.invalidate();
            }
            if changed_state && let Some(v) = self.tray.vpns.iter().find(|v| v.connected) {
                let _ =
                    door_sys::write(format!("DOOR_P15 TRAY VPN CONNECTED {}\n", v.name).as_bytes());
            }
        }
    }
    /// The page's button: connect the expanded connection, or disconnect it.
    pub(super) fn vpn_toggle(&mut self, index: u8) {
        let (Some((_, _, uid)), Some(entry)) = (
            self.session,
            self.tray.vpns.get(usize::from(index)).cloned(),
        ) else {
            return;
        };
        let Some(link) = self.tray.network_link.as_mut() else {
            return;
        };
        let result = if entry.connected || entry.connecting {
            link.vpn_disconnect(uid, &entry.name)
        } else {
            link.vpn_connect(uid, &entry.name)
        };
        let _ = door_sys::write(
            format!(
                "DOOR_P15 TRAY VPN {} {} ok={}\n",
                if entry.connected || entry.connecting {
                    "DISCONNECT"
                } else {
                    "CONNECT"
                },
                entry.name,
                u8::from(result.is_ok())
            )
            .as_bytes(),
        );
        // Show the new state at once.
        self.tray.vpn_at = 0;
    }
    /// A row: that connection becomes the expanded one.
    pub(super) fn vpn_select(&mut self, index: u8) {
        let index = usize::from(index);
        if index < self.tray.vpns.len() {
            self.tray.vpns[..=index].rotate_right(1);
            self.engine.invalidate();
        }
    }
}
