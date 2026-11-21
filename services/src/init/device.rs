//! Rename this PC (`door_service_protocol::device::SET_NAME`): the device
//! name is machine-wide (HKLM), which only system subjects may write. Init
//! accepts the request from the signed-in session's Settings app only,
//! validates the name again and writes it with its own system access.
use super::*;
use door_service_client::{RegistryRoot, RegistryValue, RegistryView, Storage};
use door_service_protocol::{application, device};

/// The caller is the live session's Settings app: an authenticated session
/// peer of the current session, Workspace manifest, and the exact process
/// and instance Init launched in the Settings slot.
pub(super) fn settings_caller(
    peer: &Identity,
    session_id: u64,
    clients: &[Option<(u64, Handle)>],
    instances: &[u64],
) -> bool {
    let Some(slot) = application::desktop_index(3) else {
        return false;
    };
    session_id != 0
        && peer.session == session_id
        && peer.manifest == door_abi::platform::manifest::WORKSPACE
        && clients
            .get(slot)
            .copied()
            .flatten()
            .is_some_and(|(pid, _)| pid == peer.pid)
        && instances.get(slot) == Some(&peer.instance)
        && peer.instance != 0
}
/// The validated name in `body` (UTF-8), or why it is refused.
pub(super) fn parse(body: &[u8]) -> Result<device::Name, Code> {
    let text = core::str::from_utf8(body).map_err(|_| Code::InvalidRequest)?;
    device::Name::parse(text).map_err(|_| Code::InvalidRequest)
}
fn units<const N: usize>(text: &str) -> ([u16; N], usize) {
    let mut out = [0; N];
    let mut len = 0;
    for (slot, unit) in out.iter_mut().zip(text.encode_utf16()) {
        *slot = unit;
        len += 1;
    }
    (out, len)
}
fn code(error: door_service_client::Error) -> Code {
    match error {
        door_service_client::Error::Service(code) => code,
        door_service_client::Error::Transport(status) => server::status(status),
        _ => Code::Io,
    }
}
/// Create `HKLM\Software\Doors\ComputerName` if needed and store the name.
fn store(port: Handle, name: &device::Name) -> Result<(), Code> {
    let mut storage = Storage::connect_port(port).map_err(code)?;
    let (path, path_len) = units::<48>(device::KEY_PATH);
    let key = storage
        .registry_open(
            RegistryRoot::LocalMachine,
            RegistryView::Bits64,
            &path[..path_len],
            3,
            true,
        )
        .map_err(code)?;
    let mut data = [0; device::REGISTRY_BYTES];
    let len = name.registry_data(&mut data);
    let (value, value_len) = units::<16>(device::VALUE);
    let written = RegistryValue::new(1, &data[..len])
        .and_then(|v| storage.registry_set(&key, &value[..value_len], v))
        .and_then(|()| storage.flush());
    let closed = storage.registry_close(key);
    written.map_err(code)?;
    closed.map_err(code)
}
impl Registry {
    pub(super) fn device_name_request(
        &mut self,
        peer: Identity,
        body: &[u8],
        transfers: &[Transfer],
    ) -> Response {
        if self.session == 0
            || !server::session_peer(peer)
            || !transfers.is_empty()
            || !settings_caller(
                &peer,
                self.session_id,
                &self.gui_clients,
                &self.gui_instances,
            )
        {
            return Response::error(Code::Denied);
        }
        let name = match parse(body) {
            Ok(name) => name,
            Err(code) => return Response::error(code),
        };
        match store(self.ports[0], &name) {
            Ok(()) => {
                line!("DOOR_P14 DEVICE_NAME saved len={}", name.as_str().len());
                Response::empty()
            }
            Err(code) => {
                line!("DOOR_P14 DEVICE_NAME failed code={code:?}");
                Response::error(code)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn peer(pid: u64, instance: u64, session: u64, manifest: u64) -> Identity {
        Identity {
            pid,
            instance,
            session,
            manifest,
            ..Identity::default()
        }
    }
    #[test]
    fn only_the_live_sessions_settings_app_may_rename_the_pc() {
        use door_abi::platform::manifest::{WINDOWS_GUI, WORKSPACE};
        let slot = application::desktop_index(3).unwrap();
        let mut clients = [None; application::DESKTOP_APPS + 1];
        let mut instances = [0; application::DESKTOP_APPS];
        clients[slot] = Some((40, 7));
        instances[slot] = 9;
        let ok = |p: Identity, c: &[Option<(u64, Handle)>], i: &[u64]| settings_caller(&p, 5, c, i);
        assert!(ok(peer(40, 9, 5, WORKSPACE), &clients, &instances));
        assert!(
            !ok(peer(41, 9, 5, WORKSPACE), &clients, &instances),
            "another process"
        );
        assert!(
            !ok(peer(40, 8, 5, WORKSPACE), &clients, &instances),
            "a stale instance"
        );
        assert!(
            !ok(peer(40, 9, 6, WORKSPACE), &clients, &instances),
            "another session"
        );
        assert!(
            !ok(peer(40, 9, 5, WINDOWS_GUI), &clients, &instances),
            "another manifest"
        );
        assert!(!settings_caller(
            &peer(40, 9, 0, WORKSPACE),
            0,
            &clients,
            &instances
        ));
        // File Explorer (slot of command 1) is not the Settings app.
        let explorer = application::desktop_index(1).unwrap();
        clients[explorer] = Some((50, 8));
        instances[explorer] = 3;
        assert!(!ok(peer(50, 3, 5, WORKSPACE), &clients, &instances));
        clients[slot] = None;
        assert!(
            !ok(peer(40, 9, 5, WORKSPACE), &clients, &instances),
            "Settings not running"
        );
    }
    #[test]
    fn request_bodies_are_validated_like_rename_this_pc() {
        assert_eq!(parse(b"Bob-Laptop").map(|n| n.to_string_len()), Ok(10));
        assert_eq!(parse(b""), Err(Code::InvalidRequest));
        assert_eq!(parse(b"my pc"), Err(Code::InvalidRequest));
        assert_eq!(parse(b"1234"), Err(Code::InvalidRequest));
        assert_eq!(parse(b"ABCDEFGHIJKLMNOP"), Err(Code::InvalidRequest));
        assert_eq!(parse(&[0xff, 0x41]), Err(Code::InvalidRequest));
    }
    trait Len {
        fn to_string_len(&self) -> usize;
    }
    impl Len for device::Name {
        fn to_string_len(&self) -> usize {
            self.as_str().len()
        }
    }
}
