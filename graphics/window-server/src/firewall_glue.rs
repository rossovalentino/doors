//! Windows Defender Firewall in the shell: door.network queues programs it
//! blocked from listening (no rule names them); the server asks for the
//! next one every second and shows "Doors Security Alert". Allow access
//! goes through User Account Control, then Init stores allow rules; Cancel
//! stores block rules at once, as Windows does. Settings' firewall page
//! sends its changes here for the same approval (`window::FIREWALL_CHANGE`).
use super::*;
use door_service_protocol::firewall::change;
use door_window::firewall_alert::{Alert, Answer};

/// The program Windows names on the prompt (the firewall's control panel).
const PROGRAM: &str = "Doors Defender Firewall";
const LOCATION: &str = "C:\\Windows\\System32\\FirewallControlPanel.dll";

impl Desktop {
    pub(super) fn firewall_tick(&mut self, now: u64) {
        if now < self.shell.firewall_at {
            return;
        }
        self.shell.firewall_at = now.saturating_add(100);
        if self.session.is_none()
            || self.shell.locked
            || self.shell.firewall_alert.is_some()
            || self.shell.uac.is_some()
        {
            return;
        }
        let Some(link) = self.tray.network_link.as_mut() else {
            return;
        };
        let mut path = [0u8; 520];
        let Ok(Some((pid, len))) = link.firewall_alert(&mut path) else {
            return;
        };
        let Ok(path) = core::str::from_utf8(&path[..len]) else {
            return;
        };
        let alert = Alert::new(pid, path);
        let scale = self.engine.scale();
        let g = door_window::firewall_alert::geometry(self.prompt_area());
        let r = |r: g::Rect| {
            let r = door_window::paint::physical(r, scale);
            alloc::format!("{},{},{},{}", r.x(), r.y(), r.width(), r.height())
        };
        let _ = door_sys::write(
            format!(
                "DOOR_P15 FIREWALL ALERT program={} pid={pid} allow={} cancel={}\n",
                alert.path,
                r(g.allow),
                r(g.cancel)
            )
            .as_bytes(),
        );
        // It comes to the front with the keyboard, as Windows' alert does.
        self.shell.firewall_alert = Some(alert);
        self.shell.firewall_alert_focused = true;
        self.engine.invalidate();
    }
    pub(super) fn firewall_answer(&mut self, answer: Answer) {
        let Some(alert) = self.shell.firewall_alert.take() else {
            return;
        };
        self.shell.firewall_alert_focused = false;
        self.engine.invalidate();
        let (allow, profiles) = match answer {
            Answer::Allow(p) => (1, p),
            Answer::Cancel(p) => (0, p),
        };
        let mut body = alloc::vec![change::PROGRAM, allow, profiles];
        body.extend_from_slice(alert.path.as_bytes());
        let _ = door_sys::write(
            format!(
                "DOOR_P15 FIREWALL ANSWER {} profiles={profiles} program={}\n",
                if allow == 1 { "allow" } else { "cancel" },
                alert.path
            )
            .as_bytes(),
        );
        if allow == 1 {
            self.firewall_consent(body);
        } else {
            self.firewall_store(&body);
        }
    }
    /// Administrator approval for a firewall change.
    fn firewall_consent(&mut self, body: Vec<u8>) {
        self.ask_consent(door_window::uac::Consent::new(
            PROGRAM,
            Some(door_window::uac::PUBLISHER),
            LOCATION,
            door_window::uac::Icon::App(door_window::apps::App::Settings),
            door_window::uac::Then::Firewall(body),
        ));
    }
    /// Init stores the change and hands the policy to door.network.
    pub(super) fn firewall_store(&mut self, body: &[u8]) {
        let result =
            door_service_client::Firewall::connect(self.registry).and_then(|mut f| f.change(body));
        let _ = door_sys::write(
            format!(
                "DOOR_P15 FIREWALL STORED kind={} ok={}\n",
                body.first().copied().unwrap_or(0),
                u8::from(result.is_ok())
            )
            .as_bytes(),
        );
    }
    /// `window::FIREWALL_CHANGE` from Settings.
    pub(super) fn firewall_change_request(
        &mut self,
        manifest: u64,
        b: &[u8],
    ) -> Result<Response, Code> {
        if manifest != door_abi::platform::manifest::WORKSPACE {
            return Err(Code::Denied);
        }
        let valid = match b {
            [change::PROFILES, which, byte] => *which & !7 == 0 && *which != 0 && *byte & !0xf == 0,
            [change::RESTORE] => true,
            [change::PROGRAM, allow, which, path @ ..] => {
                *allow <= 1
                    && *which & !7 == 0
                    && *which != 0
                    && !path.is_empty()
                    && path.len() <= 260
                    && core::str::from_utf8(path).is_ok_and(|p| !p.chars().any(char::is_control))
            }
            _ => false,
        };
        if !valid {
            return Err(Code::InvalidRequest);
        }
        if self.shell.uac.is_some() || self.session.is_none() || self.shell.locked {
            return Err(Code::WouldBlock);
        }
        self.firewall_consent(b.to_vec());
        Ok(Response::empty())
    }
}
