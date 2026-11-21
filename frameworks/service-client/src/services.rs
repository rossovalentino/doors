//! Client of the Service Control Manager Init hosts (`door_service_protocol::
//! scm`): status, start and control for advapi32 and the shells, and a
//! service process's side (registration, status, controls). Replies carry
//! a Win32 code, so these return `Result<_, u32>` (1063,
//! ERROR_FAILED_SERVICE_CONTROLLER_CONNECT, when Init can't be reached).
use crate::rpc::{Connection, Result};
use door_abi::{Handle, Transfer, rights};
use door_service_protocol::{
    manifest,
    scm::{self, Status},
};

/// One LIST entry: the service's name, state and process.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ServiceEntry {
    name: [u8; scm::NAME_BYTES],
    len: usize,
    pub state: u32,
    pub pid: u32,
}
impl ServiceEntry {
    pub fn name(&self) -> &str {
        core::str::from_utf8(&self.name[..self.len]).unwrap_or("")
    }
}
/// One page of LIST: its entries and the total count.
pub struct ServicePage {
    pub entries: [Option<ServiceEntry>; scm::PAGE],
    pub total: usize,
}

pub struct Services {
    connection: Connection,
}
const NOT_CONNECTED: u32 = 1063;
impl Services {
    pub fn connect(registry: Handle) -> Result<Self> {
        Ok(Self {
            connection: Connection::connect(registry, manifest::INIT)?,
        })
    }
    /// A request; the reply's payload after its Win32 code, into `out`
    /// (its length).
    fn call(
        &mut self,
        op: u32,
        prefix: &[u8],
        name: &str,
        transfers: &[Transfer],
        out: &mut [u8],
    ) -> core::result::Result<usize, u32> {
        let mut body = [0u8; 1 + scm::ARGUMENT_BYTES + scm::NAME_BYTES];
        if !name.is_empty() && scm::name(name.as_bytes()).is_none() {
            return Err(87);
        }
        let len = prefix.len() + name.len();
        if len > body.len() {
            return Err(87);
        }
        body[..prefix.len()].copy_from_slice(prefix);
        body[prefix.len()..len].copy_from_slice(name.as_bytes());
        let reply = self
            .connection
            .call(op, &body[..len], transfers)
            .map_err(|_| NOT_CONNECTED)?;
        let data = reply.data().map_err(|_| NOT_CONNECTED)?;
        let code = data
            .get(..4)
            .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
            .ok_or(NOT_CONNECTED)?;
        if code != 0 {
            return Err(code);
        }
        let payload = &data[4..];
        let n = payload.len().min(out.len());
        out[..n].copy_from_slice(&payload[..n]);
        Ok(n)
    }
    fn status(&mut self, op: u32, prefix: &[u8], name: &str) -> core::result::Result<Status, u32> {
        let mut out = [0u8; scm::STATUS_BYTES];
        let n = self.call(op, prefix, name, &[], &mut out)?;
        Status::decode(&out[..n]).ok_or(NOT_CONNECTED)
    }
    pub fn query(&mut self, name: &str) -> core::result::Result<Status, u32> {
        self.status(scm::QUERY, &[], name)
    }
    /// StartService; `directory` is the caller's working-folder authority
    /// (the service runs as the session's user); 0: the desktop's.
    pub fn start(&mut self, name: &str, directory: Handle) -> core::result::Result<Status, u32> {
        let mut out = [0u8; scm::STATUS_BYTES];
        let transfer = [Transfer {
            handle: directory,
            rights: rights::READ | rights::QUERY | rights::DUPLICATE | rights::TRANSFER,
            mode: 0,
        }];
        let transfers: &[Transfer] = if directory == 0 { &[] } else { &transfer };
        let n = self.call(scm::START, &[], name, transfers, &mut out)?;
        Status::decode(&out[..n]).ok_or(NOT_CONNECTED)
    }
    /// StartService with `arguments` (each NUL-terminated, at most
    /// `scm::ARGUMENT_BYTES`): the service's ServiceMain gets them after
    /// its name.
    pub fn start_with(
        &mut self,
        name: &str,
        arguments: &[u8],
    ) -> core::result::Result<Status, u32> {
        if !scm::arguments_valid(arguments) {
            return Err(87);
        }
        if arguments.is_empty() {
            return self.start(name, 0);
        }
        let mut prefix = [0u8; 1 + scm::ARGUMENT_BYTES];
        prefix[0] = arguments.len() as u8;
        prefix[1..1 + arguments.len()].copy_from_slice(arguments);
        self.status(scm::START_ARGUMENTS, &prefix[..1 + arguments.len()], name)
    }
    /// From the service's own process: the arguments its StartService
    /// passed (each NUL-terminated), once.
    pub fn arguments(
        &mut self,
        name: &str,
        out: &mut [u8; scm::ARGUMENT_BYTES],
    ) -> core::result::Result<usize, u32> {
        self.call(scm::ARGUMENTS, &[], name, &[], out)
    }
    pub fn control(&mut self, name: &str, control: u32) -> core::result::Result<Status, u32> {
        self.control_sequenced(name, control).map(|(s, _)| s)
    }
    /// ControlService: the status after queueing and the control's
    /// sequence number (to wait for its handler with `control_state`).
    pub fn control_sequenced(
        &mut self,
        name: &str,
        control: u32,
    ) -> core::result::Result<(Status, u32), u32> {
        let mut out = [0u8; scm::STATUS_BYTES + 4];
        let n = self.call(scm::CONTROL, &control.to_le_bytes(), name, &[], &mut out)?;
        let status = Status::decode(&out[..n]).ok_or(NOT_CONNECTED)?;
        let sequence = out
            .get(scm::STATUS_BYTES..n)
            .filter(|b| b.len() == 4)
            .map_or(0, |b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]));
        Ok((status, sequence))
    }
    /// Where control `sequence` of `name` stands (`scm::CONTROL_*`).
    pub fn control_state(&mut self, name: &str, sequence: u32) -> core::result::Result<u8, u32> {
        let mut out = [0u8; 1];
        let n = self.call(
            scm::CONTROL_STATE,
            &sequence.to_le_bytes(),
            name,
            &[],
            &mut out,
        )?;
        (n == 1).then_some(out[0]).ok_or(NOT_CONNECTED)
    }
    /// From the service's process: its handler returned for `sequence`.
    pub fn control_done(&mut self, name: &str, sequence: u32) -> core::result::Result<(), u32> {
        self.call(
            scm::CONTROL_DONE,
            &sequence.to_le_bytes(),
            name,
            &[],
            &mut [],
        )
        .map(|_| ())
    }
    /// The registry changed for `name` (create, delete, configuration).
    pub fn reload(&mut self, name: &str) -> core::result::Result<(), u32> {
        self.call(scm::RELOAD, &[], name, &[], &mut []).map(|_| ())
    }
    /// The services from index `first`.
    pub fn list(&mut self, first: usize) -> core::result::Result<ServicePage, u32> {
        let mut out = [0u8; 240];
        let n = self.call(scm::LIST, &(first as u16).to_le_bytes(), "", &[], &mut out)?;
        let data = &out[..n];
        if data.len() < 4 {
            return Err(NOT_CONNECTED);
        }
        let total = usize::from(u16::from_le_bytes([data[0], data[1]]));
        let count = usize::from(u16::from_le_bytes([data[2], data[3]]));
        let mut page = ServicePage {
            entries: [None; scm::PAGE],
            total,
        };
        let mut at = 4;
        for slot in page.entries.iter_mut().take(count.min(scm::PAGE)) {
            let len = usize::from(*data.get(at).ok_or(NOT_CONNECTED)?);
            let name = data.get(at + 1..at + 1 + len).ok_or(NOT_CONNECTED)?;
            if len > scm::NAME_BYTES || core::str::from_utf8(name).is_err() {
                return Err(NOT_CONNECTED);
            }
            let state = u32::from(*data.get(at + 1 + len).ok_or(NOT_CONNECTED)?);
            let pid = data
                .get(at + 2 + len..at + 6 + len)
                .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
                .ok_or(NOT_CONNECTED)?;
            let mut entry = ServiceEntry {
                name: [0; scm::NAME_BYTES],
                len,
                state,
                pid,
            };
            entry.name[..len].copy_from_slice(name);
            *slot = Some(entry);
            at += 6 + len;
        }
        Ok(page)
    }
    /// Every service, one page at a time, to `f`.
    pub fn each(&mut self, mut f: impl FnMut(&ServiceEntry)) -> core::result::Result<(), u32> {
        let mut first = 0;
        loop {
            let page = self.list(first)?;
            let mut got = 0;
            for e in page.entries.iter().flatten() {
                f(e);
                got += 1;
            }
            first += got;
            if got == 0 || first >= page.total {
                return Ok(());
            }
        }
    }
    /// StartServiceCtrlDispatcher: this process connects as the service
    /// the manager started it for; that service's name into `out` (an
    /// own-process service's table name is ignored, as on Windows). Its
    /// length.
    pub fn register(
        &mut self,
        name: &str,
        out: &mut [u8; scm::NAME_BYTES],
    ) -> core::result::Result<usize, u32> {
        let n = self.call(scm::REGISTER, &[], name, &[], out)?;
        if n == 0 || core::str::from_utf8(&out[..n]).is_err() {
            return Err(NOT_CONNECTED);
        }
        Ok(n)
    }
    pub fn set_status(&mut self, name: &str, status: &Status) -> core::result::Result<(), u32> {
        self.call(scm::SET_STATUS, &status.encode(), name, &[], &mut [])
            .map(|_| ())
    }
    /// The next control queued for `name`, if any: (control, sequence).
    pub fn next_control(&mut self, name: &str) -> core::result::Result<Option<(u32, u32)>, u32> {
        let mut out = [0u8; 8];
        let n = self.call(scm::NEXT_CONTROL, &[], name, &[], &mut out)?;
        Ok((n == 8).then(|| {
            (
                u32::from_le_bytes([out[0], out[1], out[2], out[3]]),
                u32::from_le_bytes([out[4], out[5], out[6], out[7]]),
            )
        }))
    }
}
