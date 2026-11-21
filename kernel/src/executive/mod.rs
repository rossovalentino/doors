//! Allocation-free object authority. The executive lock serializes transactions;
//! it must never be held while changing page tables or waiting for another CPU.
#[cfg(test)]
extern crate std;
mod channel;
#[cfg(test)]
mod channel_tests;
mod platform;
#[cfg(test)]
mod platform_tests;
mod port;
#[cfg(test)]
mod tests;

use channel::Channel;
pub use channel::{Message, ReceiveInfo};
use door_abi::{HandleKind, Status, rights};
pub use platform::{Authority, PlatformLease, WaitLease};

/// Owner decision of 13 November 2026 (13 before; the first step of P18's
/// capacity pass): the kernel arenas (`process/vm.rs` asserts them), the
/// kernel alias and allocation records and the telemetry ABI are sized for
/// 32. The desktop's session (Init, five services, the window server, the
/// search host and the IME) uses nine, leaving 23 for apps and tools; when
/// the table is full Start says so instead of launching
/// (`Desktop::blocked_tools`).
pub const MAX_PROCESSES: usize = 32;
/// Kernel handles one process holds (P18 M0h: 64 -> 512; a .NET process
/// keeps events, semaphores and a handle per thread, PowerShell hundreds).
/// Each process's table lives beside the process record
/// (`Executive::handles`), cleared in place.
pub const MAX_HANDLES: usize = 512;
/// Handle values start here and step by `HANDLE_STRIDE`, as Windows'
/// handles are multiples of four: a Windows program may set the low bits
/// (an OVERLAPPED's hEvent with bit 0 set keeps its completion off the
/// port), and the values stay clear of the compatibility layer's own small
/// handles (console 0x100.., registry keys 0x4000..).
pub const HANDLE_FIRST: u64 = 0x1_0000;
pub const HANDLE_STRIDE: u64 = 4;
/// Kernel objects machine-wide (P18 M0h: 256 -> 2048).
pub const MAX_OBJECTS: usize = 2048;
const MAX_CHANNELS: usize = 40;
pub const MAX_SECTIONS: usize = 32;
/// Rights of every handle to a kernel-retained resource section.
pub const RESOURCE_RIGHTS: u64 = rights::QUERY | rights::READ | rights::MAP;
/// Kernel-retained resource sections: the application asset pack and the
/// on-demand fonts and IME dictionaries (`door_abi::platform::resource`).
pub const MAX_RESOURCES: usize = 1 + door_abi::platform::resource::PATHS.len();
pub type Result<T> = core::result::Result<T, Status>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProcessId {
    slot: usize,
    pid: u64,
}
impl ProcessId {
    pub const fn slot(self) -> usize {
        self.slot
    }
    pub const fn raw(self) -> u64 {
        self.pid
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ObjectId(u64);
#[derive(Clone, Copy)]
struct Grant {
    object: ObjectId,
    rights: u64,
    value: u64,
}
#[derive(Clone, Copy)]
struct Process {
    id: ProcessId,
    object: ObjectId,
    exit: Option<i64>,
    diagnostic_remaining: usize,
    authority: Authority,
    session: Option<ObjectId>,
}
#[derive(Clone, Copy)]
enum Kind {
    Process {
        pid: u64,
        exit: Option<i64>,
        wait: Option<crate::task::ObjectId>,
        /// A 32-bit (x86) Windows process: kept on the object so its
        /// machine is known through a handle after it exits.
        x86: bool,
    },
    Section {
        key: u64,
        bytes: u64,
        maximum: u64,
    },
    /// A Windows section (P18, CreateFileMapping): sparse, demand-filled
    /// pages the VM owns (`Vm::win_section_*`), mapped anywhere in a
    /// Windows process's reservations by any number of views.
    WindowsSection {
        key: u64,
        bytes: u64,
        maximum: u64,
    },
    Channel {
        pair: usize,
        side: usize,
        owner: ProcessId,
    },
    Platform(platform::Resource),
    Port {
        slot: usize,
    },
}
impl Kind {
    fn abi(self) -> HandleKind {
        match self {
            Self::Process { .. } => HandleKind::Process,
            Self::Section { .. } | Self::WindowsSection { .. } => HandleKind::Section,
            Self::Channel { .. } => HandleKind::Channel,
            Self::Platform(resource) => resource.kind(),
            Self::Port { .. } => HandleKind::Port,
        }
    }
}
#[derive(Clone, Copy)]
struct Object {
    id: ObjectId,
    refs: u32,
    kind: Kind,
}

/// An explicit reference held across a VM operation. It cannot be duplicated by
/// copying, and must be released after a mapping is removed or operation fails.
#[derive(Debug)]
pub struct Lease {
    object: ObjectId,
}
#[derive(Debug)]
pub struct SectionLease {
    pub key: u64,
    pub bytes: u64,
    pub rights: u64,
    pub lease: Lease,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Inspection {
    pub kind: HandleKind,
    pub rights: u64,
    pub object: u64,
    pub generation: u64,
}

pub struct Executive {
    processes: [Option<Process>; MAX_PROCESSES],
    /// Each process slot's handle table.
    handles: [[Option<Grant>; MAX_HANDLES]; MAX_PROCESSES],
    objects: [Option<Object>; MAX_OBJECTS],
    channels: [Option<Channel>; MAX_CHANNELS],
    next_pid: u64,
    next_object: u64,
    next_handle: u64,
    retired_sections: [Option<u64>; MAX_SECTIONS],
    /// Windows sections whose last handle closed: the VM frees each once
    /// its last view is gone.
    retired_windows_sections: [Option<u64>; MAX_OBJECTS],
    ports: [Option<port::Port>; port::MAX_PORTS],
    sessions: [Option<platform::Session>; platform::MAX_SESSIONS],
    next_session: u64,
    retired_waits: [Option<crate::task::ObjectId>; MAX_OBJECTS],
    resources: [Option<(u64, ObjectId)>; MAX_RESOURCES],
}
impl Default for Executive {
    fn default() -> Self {
        Self::new()
    }
}
impl Executive {
    /// A fresh executive on the heap for host tests: the tables are too large
    /// to build on a test thread's stack, so a compile-time template is
    /// copied into place.
    #[cfg(test)]
    pub fn boxed() -> std::boxed::Box<Self> {
        struct Template(Executive);
        // SAFETY: read-only plain data.
        unsafe impl Sync for Template {}
        static TEMPLATE: Template = Template(Executive::new());
        let mut fresh = std::boxed::Box::<Self>::new_uninit();
        // SAFETY: a bitwise copy of a const-built value that owns no
        // resources fully initializes the allocation.
        unsafe {
            core::ptr::copy_nonoverlapping(&TEMPLATE.0, fresh.as_mut_ptr(), 1);
            fresh.assume_init()
        }
    }
    pub const fn new() -> Self {
        Self {
            processes: [None; MAX_PROCESSES],
            handles: [[None; MAX_HANDLES]; MAX_PROCESSES],
            objects: [None; MAX_OBJECTS],
            channels: [None; MAX_CHANNELS],
            next_pid: 1,
            next_object: 1,
            next_handle: HANDLE_FIRST,
            retired_sections: [None; MAX_SECTIONS],
            retired_windows_sections: [None; MAX_OBJECTS],
            ports: [None; port::MAX_PORTS],
            sessions: [None; platform::MAX_SESSIONS],
            next_session: 1,
            retired_waits: [None; MAX_OBJECTS],
            resources: [None; MAX_RESOURCES],
        }
    }
    fn process(&self, id: ProcessId) -> Result<&Process> {
        self.processes
            .get(id.slot)
            .and_then(Option::as_ref)
            .filter(|p| p.id == id)
            .ok_or(Status::NotFound)
    }
    fn process_mut(&mut self, id: ProcessId) -> Result<&mut Process> {
        self.processes
            .get_mut(id.slot)
            .and_then(Option::as_mut)
            .filter(|p| p.id == id)
            .ok_or(Status::NotFound)
    }
    /// A live process's handle table.
    fn table(&self, id: ProcessId) -> Result<&[Option<Grant>; MAX_HANDLES]> {
        self.live(id)?;
        Ok(&self.handles[id.slot])
    }
    fn live(&self, id: ProcessId) -> Result<&Process> {
        let p = self.process(id)?;
        if p.exit.is_some() {
            Err(Status::Canceled)
        } else {
            Ok(p)
        }
    }
    fn object(&self, id: ObjectId) -> &Object {
        self.objects
            .iter()
            .flatten()
            .find(|o| o.id == id)
            .expect("live object reference")
    }
    fn object_mut(&mut self, id: ObjectId) -> &mut Object {
        self.objects
            .iter_mut()
            .flatten()
            .find(|o| o.id == id)
            .expect("live object reference")
    }
    fn object_capacity(&self, count: usize) -> Result<()> {
        if self.objects.iter().filter(|o| o.is_none()).count() < count
            || self.next_object.checked_add(count as u64).is_none()
        {
            Err(Status::ResourceExhausted)
        } else {
            Ok(())
        }
    }
    fn handle_capacity(&self, pid: ProcessId, count: usize) -> Result<()> {
        if self.table(pid)?.iter().filter(|h| h.is_none()).count() < count
            || self
                .next_handle
                .checked_add(count as u64 * HANDLE_STRIDE)
                .is_none()
        {
            Err(Status::ResourceExhausted)
        } else {
            Ok(())
        }
    }
    fn allocate_object(&mut self, kind: Kind) -> ObjectId {
        let slot = self
            .objects
            .iter()
            .position(Option::is_none)
            .expect("preflight object");
        let id = ObjectId(self.next_object);
        self.next_object += 1;
        self.objects[slot] = Some(Object { id, refs: 0, kind });
        id
    }
    fn retain(&mut self, id: ObjectId) {
        let o = self.object_mut(id);
        o.refs = o.refs.checked_add(1).expect("bounded object references");
    }
    fn install(&mut self, pid: ProcessId, object: ObjectId, rights: u64) -> u64 {
        let value = self.next_handle;
        self.next_handle += HANDLE_STRIDE;
        self.retain(object);
        let table = &mut self.handles[pid.slot];
        let slot = table
            .iter()
            .position(Option::is_none)
            .expect("preflight handle");
        table[slot] = Some(Grant {
            object,
            rights,
            value,
        });
        value
    }
    fn grant(&self, pid: ProcessId, handle: u64, needed: u64) -> Result<Grant> {
        let grant = self
            .table(pid)?
            .iter()
            .flatten()
            .find(|h| h.value == handle)
            .copied()
            .ok_or(Status::BadHandle)?;
        if grant.rights & needed != needed {
            return Err(Status::AccessDenied);
        }
        Ok(grant)
    }
    fn drop_ref(&mut self, id: ObjectId) {
        let slot = self
            .objects
            .iter()
            .position(|o| o.is_some_and(|o| o.id == id))
            .expect("owned reference");
        let o = self.objects[slot].as_mut().unwrap();
        assert!(o.refs > 0);
        o.refs -= 1;
        if o.refs != 0 {
            return;
        }
        let kind = o.kind;
        self.objects[slot] = None;
        match kind {
            Kind::Section { key, .. } => {
                let target = self
                    .retired_sections
                    .iter_mut()
                    .find(|v| v.is_none())
                    .expect("bounded section retirement");
                *target = Some(key);
            }
            Kind::WindowsSection { key, .. } => {
                *self
                    .retired_windows_sections
                    .iter_mut()
                    .find(|v| v.is_none())
                    .expect("bounded Windows section retirement") = Some(key);
            }
            Kind::Channel { pair, side, .. } => self.close_endpoint(pair, side),
            Kind::Process { wait, .. } => {
                if let Some(wait) = wait {
                    *self
                        .retired_waits
                        .iter_mut()
                        .find(|v| v.is_none())
                        .expect("process wait retirement") = Some(wait);
                }
            }
            Kind::Port { slot } => self.drop_port(slot),
            Kind::Platform(resource) => self.drop_platform(resource),
        }
    }
    pub fn create_process(&mut self) -> Result<ProcessId> {
        let slot = self
            .processes
            .iter()
            .position(Option::is_none)
            .ok_or(Status::ResourceExhausted)?;
        self.object_capacity(1)?;
        let next = self
            .next_pid
            .checked_add(1)
            .ok_or(Status::ResourceExhausted)?;
        let id = ProcessId {
            slot,
            pid: self.next_pid,
        };
        self.next_pid = next;
        let object = self.allocate_object(Kind::Process {
            pid: id.pid,
            exit: None,
            wait: None,
            x86: false,
        });
        self.retain(object);
        self.handles[slot].fill(None);
        self.processes[slot] = Some(Process {
            id,
            object,
            exit: None,
            diagnostic_remaining: door_abi::PROCESS_DIAGNOSTIC_BUDGET,
            authority: Authority::fixture(id.pid),
            session: None,
        });
        Ok(id)
    }
    pub fn process_handle(
        &mut self,
        owner: ProcessId,
        target: ProcessId,
        requested: u64,
    ) -> Result<u64> {
        if requested & !(rights::QUERY | rights::WAIT | rights::DUPLICATE | rights::TRANSFER) != 0 {
            return Err(Status::AccessDenied);
        }
        self.handle_capacity(owner, 1)?;
        let object = self.process(target)?.object;
        Ok(self.install(owner, object, requested))
    }
    pub fn inspect(&self, pid: ProcessId, handle: u64) -> Result<Inspection> {
        let g = self.grant(pid, handle, rights::QUERY)?;
        Ok(Inspection {
            kind: self.object(g.object).kind.abi(),
            rights: g.rights,
            object: g.object.0,
            generation: handle,
        })
    }
    pub fn duplicate(&mut self, pid: ProcessId, handle: u64, requested: u64) -> Result<u64> {
        let g = self.grant(pid, handle, rights::DUPLICATE)?;
        if requested & !g.rights != 0 {
            return Err(Status::AccessDenied);
        }
        self.handle_capacity(pid, 1)?;
        Ok(self.install(pid, g.object, requested))
    }
    pub fn close(&mut self, pid: ProcessId, handle: u64) -> Result<()> {
        self.live(pid)?;
        self.close_owned(pid, handle)
    }
    fn close_owned(&mut self, pid: ProcessId, handle: u64) -> Result<()> {
        self.process(pid)?;
        let table = &mut self.handles[pid.slot];
        let slot = table
            .iter()
            .position(|g| g.is_some_and(|g| g.value == handle))
            .ok_or(Status::BadHandle)?;
        let object = table[slot].take().unwrap().object;
        self.drop_ref(object);
        Ok(())
    }
    pub fn exit(&mut self, pid: ProcessId, code: i64) -> Result<()> {
        let object = self.live(pid)?.object;
        self.close_owned_ports(pid);
        self.process_mut(pid)?.exit = Some(code);
        if let Kind::Process { exit, .. } = &mut self.object_mut(object).kind {
            *exit = Some(code);
        }
        loop {
            self.process(pid)?;
            let handle = self.handles[pid.slot]
                .iter()
                .flatten()
                .next()
                .map(|g| g.value);
            if let Some(h) = handle {
                self.close_owned(pid, h)?;
            } else {
                break;
            }
        }
        Ok(())
    }
    /// Runtime calls only after all threads and address-space references retired.
    pub fn reap(&mut self, pid: ProcessId) -> Result<i64> {
        let p = *self.process(pid)?;
        let code = p.exit.ok_or(Status::WouldBlock)?;
        self.processes[pid.slot] = None;
        self.drop_ref(p.object);
        if let Some(session) = p.session {
            self.drop_ref(session);
        }
        Ok(code)
    }
    pub fn exit_code(&self, pid: ProcessId) -> Result<Option<i64>> {
        Ok(self.process(pid)?.exit)
    }
    pub fn charge_diagnostic(&mut self, pid: ProcessId, bytes: usize) -> Result<()> {
        self.live(pid)?;
        if bytes > door_abi::MAX_DIAGNOSTIC {
            return Err(Status::InvalidArgument);
        }
        let p = self.process_mut(pid)?;
        let remaining = p
            .diagnostic_remaining
            .checked_sub(bytes)
            .ok_or(Status::ResourceExhausted)?;
        p.diagnostic_remaining = remaining;
        Ok(())
    }
    pub fn wait_process(&self, pid: ProcessId, handle: u64) -> Result<(u64, Option<i64>)> {
        let g = self.grant(pid, handle, rights::WAIT)?;
        if let Kind::Process { pid, exit, .. } = self.object(g.object).kind {
            Ok((pid, exit))
        } else {
            Err(Status::InvalidArgument)
        }
    }
    pub fn create_section(
        &mut self,
        pid: ProcessId,
        key: u64,
        bytes: u64,
        requested: u64,
    ) -> Result<u64> {
        if key == 0
            || bytes == 0
            || bytes & 4095 != 0
            || requested
                & !(rights::QUERY
                    | rights::DUPLICATE
                    | rights::TRANSFER
                    | rights::READ
                    | rights::WRITE
                    | rights::MAP)
                != 0
            || requested & rights::READ == 0
        {
            return Err(Status::InvalidArgument);
        }
        if self
            .objects
            .iter()
            .flatten()
            .filter(|o| matches!(o.kind, Kind::Section { .. }))
            .count()
            + self.retired_sections.iter().flatten().count()
            >= MAX_SECTIONS
        {
            return Err(Status::ResourceExhausted);
        }
        if self
            .objects
            .iter()
            .flatten()
            .any(|o| matches!(o.kind,Kind::Section{key:k,..} if k==key))
            || self.retired_sections.contains(&Some(key))
        {
            return Err(Status::AlreadyExists);
        }
        self.object_capacity(1)?;
        self.handle_capacity(pid, 1)?;
        let object = self.allocate_object(Kind::Section {
            key,
            bytes,
            maximum: requested,
        });
        Ok(self.install(pid, object, requested))
    }
    /// Immutable boot resource shared by every process it is granted to (the
    /// application asset pack). The executive holds one pinning reference
    /// until `release_resource_section`, so the section outlives each process
    /// it is granted to and its frames are one copy however many processes map
    /// it. Handles carry exactly `RESOURCE_RIGHTS`: never WRITE, DUPLICATE or
    /// TRANSFER.
    pub fn create_resource_section(&mut self, key: u64, bytes: u64) -> Result<()> {
        if key == 0 || bytes == 0 || bytes & 4095 != 0 {
            return Err(Status::InvalidArgument);
        }
        if self
            .objects
            .iter()
            .flatten()
            .any(|o| matches!(o.kind,Kind::Section{key:k,..} if k==key))
            || self.retired_sections.contains(&Some(key))
        {
            return Err(Status::AlreadyExists);
        }
        let slot = self
            .resources
            .iter()
            .position(Option::is_none)
            .ok_or(Status::ResourceExhausted)?;
        if self
            .objects
            .iter()
            .flatten()
            .filter(|o| matches!(o.kind, Kind::Section { .. }))
            .count()
            + self.retired_sections.iter().flatten().count()
            >= MAX_SECTIONS
        {
            return Err(Status::ResourceExhausted);
        }
        self.object_capacity(1)?;
        let object = self.allocate_object(Kind::Section {
            key,
            bytes,
            maximum: RESOURCE_RIGHTS,
        });
        self.retain(object);
        self.resources[slot] = Some((key, object));
        Ok(())
    }
    /// A `RESOURCE_RIGHTS` handle to a pinned resource section in `pid`.
    pub fn grant_resource_section(&mut self, pid: ProcessId, key: u64) -> Result<u64> {
        let object = self
            .resources
            .iter()
            .flatten()
            .find(|(k, _)| *k == key)
            .map(|(_, object)| *object)
            .ok_or(Status::NotFound)?;
        self.handle_capacity(pid, 1)?;
        Ok(self.install(pid, object, RESOURCE_RIGHTS))
    }
    /// Drop the executive's pin (shutdown). No new grants are possible; the
    /// section retires once its last handle and view lease are gone.
    pub fn release_resource_section(&mut self, key: u64) -> Result<()> {
        let slot = self
            .resources
            .iter()
            .position(|r| r.is_some_and(|(k, _)| k == key))
            .ok_or(Status::NotFound)?;
        let (_, object) = self.resources[slot].take().expect("pinned resource");
        self.drop_ref(object);
        Ok(())
    }
    pub fn acquire_section(
        &mut self,
        pid: ProcessId,
        handle: u64,
        requested: u64,
    ) -> Result<SectionLease> {
        if requested & !(rights::READ | rights::WRITE) != 0 || requested & rights::READ == 0 {
            return Err(Status::InvalidArgument);
        }
        let g = self.grant(pid, handle, rights::MAP | requested)?;
        if let Kind::Section {
            key,
            bytes,
            maximum,
        } = self.object(g.object).kind
        {
            if requested & !maximum != 0 {
                return Err(Status::AccessDenied);
            }
            self.retain(g.object);
            Ok(SectionLease {
                key,
                bytes,
                rights: requested,
                lease: Lease { object: g.object },
            })
        } else {
            Err(Status::InvalidArgument)
        }
    }
    pub fn release_lease(&mut self, lease: Lease) {
        self.drop_ref(lease.object);
    }
    /// A Windows section (CreateFileMapping) whose pages the VM holds under
    /// `key`: a handle with `requested` rights (READ and MAP at least).
    pub fn create_windows_section(
        &mut self,
        pid: ProcessId,
        key: u64,
        bytes: u64,
        requested: u64,
    ) -> Result<u64> {
        let known = rights::QUERY
            | rights::DUPLICATE
            | rights::TRANSFER
            | rights::READ
            | rights::WRITE
            | rights::MAP;
        if key == 0
            || bytes == 0
            || bytes & 4095 != 0
            || requested & !known != 0
            || requested & (rights::READ | rights::MAP) != rights::READ | rights::MAP
        {
            return Err(Status::InvalidArgument);
        }
        self.object_capacity(1)?;
        self.handle_capacity(pid, 1)?;
        let object = self.allocate_object(Kind::WindowsSection {
            key,
            bytes,
            maximum: requested,
        });
        Ok(self.install(pid, object, requested))
    }
    /// The (key, bytes) of the Windows section `handle` names, if the handle
    /// grants MAP and `requested` (READ, WRITE) to `pid`.
    pub fn windows_section(
        &self,
        pid: ProcessId,
        handle: u64,
        requested: u64,
    ) -> Result<(u64, u64)> {
        let g = self.grant(pid, handle, rights::MAP | requested)?;
        match self.object(g.object).kind {
            Kind::WindowsSection {
                key,
                bytes,
                maximum,
            } if requested & !maximum == 0 => Ok((key, bytes)),
            Kind::WindowsSection { .. } => Err(Status::AccessDenied),
            _ => Err(Status::InvalidArgument),
        }
    }
    pub fn take_retired_windows_section(&mut self) -> Option<u64> {
        self.retired_windows_sections
            .iter_mut()
            .find(|v| v.is_some())
            .and_then(Option::take)
    }
    pub fn take_retired_section(&mut self) -> Option<u64> {
        self.retired_sections
            .iter_mut()
            .find(|v| v.is_some())
            .and_then(Option::take)
    }
    pub fn handle_count(&self, pid: ProcessId) -> Result<usize> {
        self.process(pid)?;
        Ok(self.handles[pid.slot].iter().flatten().count())
    }
    pub fn object_count(&self) -> usize {
        self.objects.iter().flatten().count()
    }
}
