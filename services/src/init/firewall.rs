//! Windows Defender Firewall's store (`door_service_protocol::firewall`):
//! the rules and profile switches in HKLM, where Windows keeps them. Init
//! reads them when door.network becomes ready and after each change, and
//! hands them over (`net::firewall` STAGE/APPLY); it names the program of
//! every process it starts (PROGRAM). Changes come from administrators
//! (elevated netsh writes the registry, then asks for RELOAD) and from the
//! window server once User Account Control approved (CHANGE).
use super::*;
use door_service_client::{Network, RegistryRoot, RegistryValue, RegistryView, Storage};
use door_service_protocol::firewall as fw;
use door_service_protocol::net::firewall as nf;

fn code(error: door_service_client::Error) -> Code {
    match error {
        door_service_client::Error::Service(code) => code,
        door_service_client::Error::Transport(status) => server::status(status),
        _ => Code::Io,
    }
}
/// `FirewallPolicy\sub` as UTF-16 units.
fn path(sub: &str) -> ([u16; 96], usize) {
    let mut out = [0; 96];
    let mut len = 0;
    for unit in fw::POLICY_KEY
        .encode_utf16()
        .chain("\\".encode_utf16())
        .chain(sub.encode_utf16())
    {
        if len == out.len() {
            break;
        }
        out[len] = unit;
        len += 1;
    }
    (out, len)
}
fn name(text: &[u8]) -> ([u16; 48], usize) {
    let mut out = [0; 48];
    let mut len = 0;
    for (slot, unit) in out
        .iter_mut()
        .zip(core::str::from_utf8(text).unwrap_or("").encode_utf16())
    {
        *slot = unit;
        len += 1;
    }
    (out, len)
}
/// UTF-16 to UTF-8 into `out` (as much as fits): bytes written.
fn utf8(units: impl Iterator<Item = u16>, out: &mut [u8]) -> usize {
    let mut len = 0;
    for c in char::decode_utf16(units) {
        let c = c.unwrap_or(char::REPLACEMENT_CHARACTER);
        if len + c.len_utf8() > out.len() {
            break;
        }
        len += c.encode_utf8(&mut out[len..]).len();
    }
    len
}
fn open(storage: &mut Storage, sub: &str, write: bool) -> Result<door_service_client::Key, Code> {
    let (p, n) = path(sub);
    storage
        .registry_open(
            RegistryRoot::LocalMachine,
            RegistryView::Bits64,
            &p[..n],
            if write { 3 } else { 1 },
            write,
        )
        .map_err(code)
}
/// The three profiles' bytes, Windows' defaults where values are absent.
fn profiles(storage: &mut Storage) -> [u8; 3] {
    let mut out = [nf::profile::DEFAULT; 3];
    for (slot, key) in out.iter_mut().zip(fw::PROFILE_KEYS) {
        let Ok(k) = open(storage, key, false) else {
            continue;
        };
        let mut values = [None; 4];
        for (value, label) in values.iter_mut().zip(fw::VALUES) {
            let (v, n) = name(label.as_bytes());
            if let Ok(r) = storage.registry_query(&k, &v[..n])
                && r.kind == 4
            {
                *value = Some(u32::from_le_bytes(r.data().try_into().unwrap_or([0; 4])));
            }
        }
        let _ = storage.registry_close(k);
        *slot = fw::profile_byte(values);
    }
    out
}
/// Hand the stored policy to door.network (port `network`): the number of
/// rules handed over.
pub(super) fn push(storage: Handle, network: Handle) -> Result<usize, Code> {
    let mut storage = Storage::connect_port(storage).map_err(code)?;
    let profiles = profiles(&mut storage);
    let mut network = Network::connect_port(network).map_err(code)?;
    network.firewall_stage(0, &[]).map_err(code)?;
    let mut offset = 0usize;
    let mut rules = 0;
    if let Ok(key) = open(&mut storage, fw::RULES, false) {
        let version = storage.registry_version(&key).unwrap_or(0);
        // "id\trule\n": a 48-unit name and a 1024-unit rule at most.
        let mut line = [0u8; 48 * 3 + 1024 * 3 + 2];
        for index in 0.. {
            let Ok(entry) = storage.registry_enumerate_value(&key, index, version) else {
                break;
            };
            if entry.value.kind != 1 {
                continue;
            }
            let mut len = utf8(entry.name.units().iter().copied(), &mut line);
            line[len] = b'\t';
            len += 1;
            let text = entry
                .value
                .data()
                .chunks_exact(2)
                .map(|b| u16::from_le_bytes([b[0], b[1]]))
                .take_while(|&u| u != 0);
            let end = line.len() - 1;
            len += utf8(text, &mut line[len..end]);
            line[len] = b'\n';
            len += 1;
            for part in line[..len].chunks(nf::PART) {
                network.firewall_stage(offset as u32, part).map_err(code)?;
                offset += part.len();
            }
            rules += 1;
        }
        let _ = storage.registry_close(key);
    }
    network.firewall_apply(profiles).map_err(code)?;
    Ok(rules)
}
/// Tell door.network which program process `pid` runs (its native path,
/// spelled as Windows names it).
pub(super) fn program(network: Handle, pid: u64, native: &str) {
    if network == 0 {
        return;
    }
    let mut spelled = [0u8; 520];
    let Some(len) = door_service_protocol::shell::windows_spelling(native, &mut spelled) else {
        return;
    };
    let Ok(text) = core::str::from_utf8(&spelled[..len]) else {
        return;
    };
    if let Ok(mut n) = Network::connect_port(network) {
        let _ = n.firewall_program(pid, text);
    }
}
/// Store one change the window server's consent covers.
fn change(storage: Handle, body: &[u8]) -> Result<(), Code> {
    let mut storage = Storage::connect_port(storage).map_err(code)?;
    match body {
        [fw::change::PROFILES, which, byte] if *which & !7 == 0 && *byte & !0xf == 0 => {
            for (i, key) in fw::PROFILE_KEYS.iter().enumerate() {
                if which & (1 << i) == 0 {
                    continue;
                }
                write_profile(&mut storage, key, *byte)?;
            }
        }
        [fw::change::PROGRAM, allow, which, path @ ..]
            if *allow <= 1 && *which & !7 == 0 && *which != 0 =>
        {
            let path = core::str::from_utf8(path).map_err(|_| Code::InvalidRequest)?;
            if path.is_empty() || path.len() > 260 {
                return Err(Code::InvalidRequest);
            }
            let key = open(&mut storage, fw::RULES, true)?;
            let mut result = Ok(());
            for protocol in [6u8, 17] {
                let mut text = [0u8; 400];
                let Some(len) = fw::program_rule(*allow == 1, protocol, *which, path, &mut text)
                else {
                    result = Err(Code::InvalidRequest);
                    break;
                };
                let mut data = [0u8; 2 * 400 + 2];
                let mut at = 0;
                for unit in core::str::from_utf8(&text[..len])
                    .unwrap_or("")
                    .encode_utf16()
                {
                    data[at..at + 2].copy_from_slice(&unit.to_le_bytes());
                    at += 2;
                }
                at += 2;
                let id = fw::program_rule_id(path, protocol);
                let (v, n) = name(&id);
                result = RegistryValue::new(1, &data[..at])
                    .and_then(|value| storage.registry_set(&key, &v[..n], value))
                    .map_err(code);
                if result.is_err() {
                    break;
                }
            }
            let _ = storage.registry_close(key);
            result?;
        }
        [fw::change::RESTORE] => {
            if let Ok(key) = open(&mut storage, fw::RULES, true) {
                // Delete from the front until nothing is left.
                let mut result = Ok(());
                for _ in 0..4096 {
                    let version = storage.registry_version(&key).unwrap_or(0);
                    let Ok(entry) = storage.registry_enumerate_value(&key, 0, version) else {
                        break;
                    };
                    result = storage
                        .registry_delete_value(&key, entry.name.units())
                        .map_err(code);
                    if result.is_err() {
                        break;
                    }
                }
                let _ = storage.registry_close(key);
                result?;
            }
            for key in fw::PROFILE_KEYS {
                write_profile(&mut storage, key, nf::profile::DEFAULT)?;
            }
        }
        _ => return Err(Code::InvalidRequest),
    }
    storage.flush().map_err(code)
}
fn write_profile(storage: &mut Storage, key: &str, byte: u8) -> Result<(), Code> {
    let k = open(storage, key, true)?;
    let mut result = Ok(());
    for (label, value) in fw::VALUES.iter().zip(fw::profile_values(byte)) {
        let (v, n) = name(label.as_bytes());
        result = storage
            .registry_set(&k, &v[..n], RegistryValue::dword(value))
            .map_err(code);
        if result.is_err() {
            break;
        }
    }
    let _ = storage.registry_close(k);
    result
}
impl Init {
    /// door.network (port `network`) is ready: hand it the policy and the
    /// programs already running.
    pub(super) fn firewall_start(&mut self, network: Handle) {
        match push(self.registry.ports[0], network) {
            Ok(rules) => line!("DOOR_P15 FIREWALL LOADED rules={rules}"),
            Err(e) => line!("DOOR_P15 FIREWALL LOAD failed code={e:?}"),
        }
        for job in self.registry.jobs.iter().flatten() {
            program(network, job.pid(), job.path());
        }
    }
}
impl Registry {
    pub(super) fn firewall_request(
        &mut self,
        peer: Identity,
        operation: u32,
        body: &[u8],
        transfers: &[Transfer],
    ) -> Response {
        let window = peer.manifest == door_abi::platform::manifest::WINDOW
            && peer.session == 0
            && peer.instance == self.window_instance
            && self.window_instance != 0;
        let session_app = self.admits(peer);
        if !transfers.is_empty() {
            return Response::error(Code::InvalidRequest);
        }
        match operation {
            fw::RELOAD if body.is_empty() && (window || session_app) => {}
            fw::CHANGE if window => {
                if let Err(e) = change(self.ports[0], body) {
                    line!("DOOR_P15 FIREWALL CHANGE failed code={e:?}");
                    return Response::error(e);
                }
                line!("DOOR_P15 FIREWALL CHANGE kind={}", body[0]);
            }
            fw::RELOAD | fw::CHANGE => return Response::error(Code::Denied),
            _ => return Response::error(Code::InvalidRequest),
        }
        if self.network == 0 {
            // Handed over when door.network starts.
            return Response::empty();
        }
        match push(self.ports[0], self.network) {
            Ok(rules) => {
                line!("DOOR_P15 FIREWALL RELOAD rules={rules}");
                Response::empty()
            }
            Err(e) => Response::error(e),
        }
    }
}
