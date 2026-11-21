//! Kernel-owned platform capabilities. Wire identities describe these records;
//! they never create authority. Every operation runs under the executive guard.
use super::*;
use crate::task::ObjectId as WaitId;

pub(super) const MAX_SESSIONS: usize = 8;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Authority {
    pub uid: u32,
    pub integrity: u32,
    pub groups: u64,
    pub session: u64,
    pub epoch: u64,
    pub manifest: u64,
    pub instance: u64,
    pub capabilities: u64,
}
impl Authority {
    pub(super) const fn fixture(_pid: u64) -> Self {
        Self {
            uid: 0,
            integrity: 0,
            groups: 0,
            session: 0,
            epoch: 0,
            manifest: 0,
            instance: 0,
            capabilities: 0,
        }
    }
}

#[derive(Clone, Copy)]
pub(super) struct Session {
    id: u64,
    object: ObjectId,
    uid: u32,
    /// The account is an administrator: its applications' tokens are the
    /// filtered ones (Administrators for deny only).
    administrator: bool,
    revoked: bool,
}

#[derive(Clone, Copy)]
pub(super) enum Resource {
    Authority {
        kind: HandleKind,
        owner: ProcessId,
    },
    Permit {
        uid: u32,
        consumed: bool,
    },
    Session {
        id: u64,
    },
    Directory {
        uid: u32,
    },
    Wait {
        kind: HandleKind,
        wait: WaitId,
    },
    Thread {
        key: u64,
        owner: ProcessId,
        wait: WaitId,
        exit: Option<i64>,
    },
}
impl Resource {
    pub(super) fn kind(self) -> HandleKind {
        match self {
            Self::Authority { kind, .. } | Self::Wait { kind, .. } => kind,
            Self::Permit { .. } => HandleKind::LogonPermit,
            Self::Session { .. } => HandleKind::Session,
            Self::Directory { .. } => HandleKind::Directory,
            Self::Thread { .. } => HandleKind::Thread,
        }
    }
}

pub struct WaitLease {
    pub wait: WaitId,
    pub kind: HandleKind,
    pub lease: Lease,
}
pub struct PlatformLease {
    pub key: u64,
    pub wait: WaitId,
    pub exit: Option<i64>,
    pub lease: Lease,
}

const COPY: u64 = rights::QUERY | rights::DUPLICATE | rights::TRANSFER;

impl Executive {
    /// Trusted loader only, before any thread or grant is made runnable.
    pub fn assign_authority(&mut self, pid: ProcessId, authority: Authority) -> Result<()> {
        if self.live(pid)?.authority.manifest != 0
            || authority.manifest == 0
            || authority.instance == 0
            || authority.uid == 0
        {
            return Err(Status::InvalidArgument);
        }
        if authority.session != 0 {
            let session = self.validate_session(authority.session, authority.epoch)?;
            if authority.uid != session.uid
                || door_abi::platform::manifest_policy(authority.manifest)
                    .is_none_or(|policy| policy.uid != 0 || policy.caps != 0)
                || authority.capabilities != 0
                || authority.integrity != 1
                // BUILTIN\Users, plus Administrators enabled when User
                // Account Control approved the launch (Init's full token)
                // and for deny only when the account is an administrator.
                || authority.groups & !door_abi::platform::group::SESSION != 0
                || authority.groups & door_abi::platform::group::USERS == 0
                || (authority.groups & door_abi::platform::group::ADMINISTRATOR_ACCOUNT != 0)
                    != session.administrator
            {
                return Err(Status::AccessDenied);
            }
            self.retain(session.object);
            self.process_mut(pid)?.session = Some(session.object);
        }
        let process = self.process_mut(pid)?;
        process.authority = authority;
        // A manifest with a larger diagnostic budget gets the difference.
        process.diagnostic_remaining += door_abi::platform::diagnostic_budget(authority.manifest)
            .saturating_sub(door_abi::PROCESS_DIAGNOSTIC_BUDGET);
        Ok(())
    }
    pub fn authority(&self, pid: ProcessId) -> Result<Authority> {
        let authority = self.live(pid)?.authority;
        self.validate_authority(authority)?;
        Ok(authority)
    }
    pub fn validate_authority(&self, authority: Authority) -> Result<()> {
        if authority.session != 0 {
            self.validate_session(authority.session, authority.epoch)?;
        }
        Ok(())
    }
    fn validate_session(&self, id: u64, epoch: u64) -> Result<Session> {
        self.sessions
            .iter()
            .flatten()
            .find(|s| s.id == id && id == epoch && !s.revoked)
            .copied()
            .ok_or(Status::AccessDenied)
    }
    /// Query a held live Session grant without creating a bootstrap process.
    pub fn session_info(&self, pid: ProcessId, handle: u64) -> Result<(u64, u32)> {
        self.authority(pid)?;
        let grant = self.grant(pid, handle, rights::QUERY)?;
        let Kind::Platform(Resource::Session { id }) = self.object(grant.object).kind else {
            return Err(Status::InvalidArgument);
        };
        let session = self.validate_session(id, id)?;
        Ok((id, session.uid))
    }
    pub fn authority_for_launch(
        &self,
        parent: ProcessId,
        grant: u64,
        manifest: u64,
        session: u64,
        instance: u64,
    ) -> Result<Authority> {
        self.require_authority(parent, grant, HandleKind::Launch)?;
        if instance == 0 {
            return Err(Status::InvalidArgument);
        }
        let policy = door_abi::platform::manifest_policy(manifest)
            .filter(|_| manifest != 1)
            .ok_or(Status::AccessDenied)?;
        let (uid, caps) = (policy.uid, policy.caps);
        let mut authority = Authority {
            uid,
            integrity: if caps == 0 { 1 } else { 2 },
            groups: if caps == 0 { 2 } else { 4 },
            session: 0,
            epoch: 0,
            manifest,
            instance,
            capabilities: caps,
        };
        if session != 0 {
            if policy.uid != 0 || policy.caps != 0 {
                return Err(Status::AccessDenied);
            }
            let g = self.grant(parent, session, rights::READ)?;
            let Kind::Platform(Resource::Session { id }) = self.object(g.object).kind else {
                return Err(Status::InvalidArgument);
            };
            let s = self.validate_session(id, id)?;
            authority.uid = s.uid;
            authority.session = id;
            authority.epoch = id;
            if s.administrator {
                authority.groups |= door_abi::platform::group::ADMINISTRATOR_ACCOUNT;
            }
        } else if policy.uid == 0 {
            return Err(Status::AccessDenied);
        }
        Ok(authority)
    }
    pub fn create_role_authority(&mut self, pid: ProcessId, kind: HandleKind) -> Result<u64> {
        let expected = match kind {
            HandleKind::Launch => 1,
            HandleKind::Block => 2,
            HandleKind::Session => 4,
            HandleKind::Console => 8,
            HandleKind::Display => 16,
            HandleKind::Network => 32,
            HandleKind::Audio => 64,
            _ => return Err(Status::InvalidArgument),
        };
        if self.authority(pid)?.capabilities & expected == 0 {
            return Err(Status::AccessDenied);
        }
        self.create_resource(
            pid,
            Resource::Authority { kind, owner: pid },
            rights::QUERY | rights::READ | rights::WRITE,
        )
    }
    pub fn require_authority(&self, pid: ProcessId, handle: u64, kind: HandleKind) -> Result<()> {
        self.authority(pid)?;
        let g = self.grant(pid, handle, rights::WRITE)?;
        match self.object(g.object).kind {
            Kind::Platform(Resource::Authority {
                kind: actual,
                owner,
            }) if actual == kind && owner == pid => Ok(()),
            _ => Err(Status::AccessDenied),
        }
    }
    /// Trusted boot loader provisioning. No user syscall can manufacture permits.
    pub fn provision_permit(&mut self, pid: ProcessId, uid: u32) -> Result<u64> {
        if self.authority(pid)?.manifest != 1
            || ![1001, 1002, door_abi::platform::SERVICE_ACCOUNT].contains(&uid)
        {
            return Err(Status::AccessDenied);
        }
        self.create_resource(
            pid,
            Resource::Permit {
                uid,
                consumed: false,
            },
            COPY | rights::READ,
        )
    }
    /// Sign out: Init (the Launch authority's holder, which held the boot
    /// permits) gets a fresh logon permit for local account `uid`, but only
    /// while no live session of that account remains, so a permit never
    /// stands beside the session it admitted.
    pub fn renew_permit(&mut self, pid: ProcessId, authority: u64, uid: u32) -> Result<u64> {
        self.require_authority(pid, authority, HandleKind::Launch)?;
        // Interactive accounts only: the service account signs in once.
        if !door_abi::platform::ACCOUNTS.contains(&uid) {
            return Err(Status::AccessDenied);
        }
        if self
            .sessions
            .iter()
            .flatten()
            .any(|s| s.uid == uid && !s.revoked)
        {
            return Err(Status::AccessDenied);
        }
        self.provision_permit(pid, uid)
    }
    pub fn create_directory(&mut self, pid: ProcessId) -> Result<u64> {
        let uid = self.authority(pid)?.uid;
        self.create_resource(pid, Resource::Directory { uid }, COPY | rights::READ)
    }
    pub fn directory_uid(&self, pid: ProcessId, handle: u64) -> Result<u32> {
        let g = self.grant(pid, handle, rights::READ)?;
        if let Kind::Platform(Resource::Directory { uid }) = self.object(g.object).kind {
            Ok(uid)
        } else {
            Err(Status::InvalidArgument)
        }
    }
    pub fn create_session(&mut self, pid: ProcessId, authority: u64, permit: u64) -> Result<u64> {
        self.create_session_typed(pid, authority, permit, 0)
    }
    /// A session for the permit's account; `administrators` (the session
    /// service's account book, bit `i` for `door_abi::platform::ACCOUNTS[i]`)
    /// says whether that account is an administrator.
    pub fn create_session_typed(
        &mut self,
        pid: ProcessId,
        authority: u64,
        permit: u64,
        administrators: u64,
    ) -> Result<u64> {
        let accounts = door_abi::platform::ACCOUNTS;
        if administrators >> accounts.len() != 0 {
            return Err(Status::InvalidArgument);
        }
        self.require_authority(pid, authority, HandleKind::Session)?;
        let grant = self.grant(pid, permit, rights::READ)?;
        let Kind::Platform(Resource::Permit {
            uid,
            consumed: false,
        }) = self.object(grant.object).kind
        else {
            return Err(Status::AccessDenied);
        };
        let slot = self
            .sessions
            .iter()
            .position(|s| s.is_none_or(|s| s.revoked))
            .ok_or(Status::ResourceExhausted)?;
        let next = self
            .next_session
            .checked_add(1)
            .ok_or(Status::ResourceExhausted)?;
        self.object_capacity(1)?;
        self.handle_capacity(pid, 1)?;
        let id = self.next_session;
        let handle = self.create_resource(
            pid,
            Resource::Session { id },
            COPY | rights::READ | rights::WRITE,
        )?;
        self.sessions[slot] = Some(Session {
            id,
            object: self.grant(pid, handle, 0)?.object,
            uid,
            // The service account is an administrator.
            administrator: uid == door_abi::platform::SERVICE_ACCOUNT
                || accounts
                    .iter()
                    .position(|&u| u == uid)
                    .is_some_and(|i| administrators & (1 << i) != 0),
            revoked: false,
        });
        self.next_session = next;
        self.object_mut(grant.object).kind = Kind::Platform(Resource::Permit {
            uid,
            consumed: true,
        });
        // A copied permit is consumed too because the flag belongs to its object.
        self.close_owned(pid, permit)
            .expect("validated consumed permit");
        Ok(handle)
    }
    pub fn revoke_session(&mut self, pid: ProcessId, authority: u64, handle: u64) -> Result<u64> {
        self.require_authority(pid, authority, HandleKind::Session)?;
        let grant = self.grant(pid, handle, rights::WRITE)?;
        let Kind::Platform(Resource::Session { id }) = self.object(grant.object).kind else {
            return Err(Status::InvalidArgument);
        };
        let session = self
            .sessions
            .iter_mut()
            .flatten()
            .find(|s| s.id == id)
            .ok_or(Status::AccessDenied)?;
        session.revoked = true;
        Ok(id)
    }
    pub fn session_processes(&self, session: u64) -> [Option<ProcessId>; MAX_PROCESSES] {
        core::array::from_fn(|i| {
            self.processes[i]
                .filter(|p| p.authority.session == session && p.exit.is_none())
                .map(|p| p.id)
        })
    }
    /// Records that `pid` is a 32-bit (x86) Windows process (at its
    /// launch's commit).
    pub fn set_process_x86(&mut self, pid: ProcessId, value: bool) -> Result<()> {
        let object = self.process(pid)?.object;
        if let Kind::Process { x86, .. } = &mut self.object_mut(object).kind {
            *x86 = value;
        }
        Ok(())
    }
    /// Whether the process `pid`'s `handle` (WAIT right) names is a 32-bit
    /// Windows process, running or exited.
    pub fn process_x86(&self, pid: ProcessId, handle: u64) -> Result<bool> {
        let g = self.grant(pid, handle, rights::WAIT)?;
        match self.object(g.object).kind {
            Kind::Process { x86, .. } => Ok(x86),
            _ => Err(Status::InvalidArgument),
        }
    }
    pub fn process_by_raw(&self, raw: u64) -> Result<ProcessId> {
        self.processes
            .iter()
            .flatten()
            .find(|p| p.id.raw() == raw)
            .map(|p| p.id)
            .ok_or(Status::NotFound)
    }
    pub fn process_target(&self, pid: ProcessId, handle: u64) -> Result<ProcessId> {
        let (target, _) = self.wait_process(pid, handle)?;
        if self.authority(pid)?.capabilities & 1 == 0 && target != pid.raw() {
            return Err(Status::AccessDenied);
        }
        let target = self.process_by_raw(target)?;
        if matches!(
            self.object(self.process(target)?.object).kind,
            Kind::Process { wait: None, .. }
        ) {
            return Err(Status::WouldBlock);
        }
        Ok(target)
    }
    /// Final launch publication gate. Runtime records must be installed under
    /// the same metadata lock, before a revoker can enumerate this process.
    /// A failed check leaves wait ownership with the construction rollback.
    pub fn set_process_wait(&mut self, pid: ProcessId, event: WaitId) -> Result<()> {
        self.authority(pid)?;
        let object = self.live(pid)?.object;
        let Kind::Process { wait, .. } = &mut self.object_mut(object).kind else {
            unreachable!()
        };
        if wait.is_some() {
            return Err(Status::AlreadyExists);
        }
        *wait = Some(event);
        Ok(())
    }
    pub fn acquire_process_wait(&mut self, pid: ProcessId, handle: u64) -> Result<PlatformLease> {
        let g = self.grant(pid, handle, rights::WAIT)?;
        let Kind::Process {
            pid: key,
            exit,
            wait: Some(wait),
            ..
        } = self.object(g.object).kind
        else {
            return Err(Status::InvalidArgument);
        };
        self.retain(g.object);
        Ok(PlatformLease {
            key,
            wait,
            exit,
            lease: Lease { object: g.object },
        })
    }
    pub fn process_result(&self, lease: &Lease) -> Result<Option<i64>> {
        if let Kind::Process { exit, .. } = self.object(lease.object).kind {
            Ok(exit)
        } else {
            Err(Status::InvalidArgument)
        }
    }
    pub fn copy_grant_to(
        &mut self,
        source: ProcessId,
        handle: u64,
        target: ProcessId,
        requested: u64,
    ) -> Result<u64> {
        let g = self.grant(source, handle, rights::TRANSFER)?;
        if requested & !g.rights != 0
            || matches!(
                self.object(g.object).kind,
                Kind::Channel { .. } | Kind::Platform(Resource::Authority { .. })
            )
        {
            return Err(Status::AccessDenied);
        }
        self.handle_capacity(target, 1)?;
        Ok(self.install(target, g.object, requested))
    }
    fn create_resource(&mut self, pid: ProcessId, resource: Resource, access: u64) -> Result<u64> {
        self.object_capacity(1)?;
        self.handle_capacity(pid, 1)?;
        let object = self.allocate_object(Kind::Platform(resource));
        Ok(self.install(pid, object, access))
    }
    pub fn create_wait(&mut self, pid: ProcessId, kind: HandleKind, wait: WaitId) -> Result<u64> {
        if !matches!(
            kind,
            HandleKind::Event | HandleKind::Mutex | HandleKind::Condition | HandleKind::Semaphore
        ) {
            return Err(Status::InvalidArgument);
        }
        self.create_resource(
            pid,
            Resource::Wait { kind, wait },
            COPY | rights::READ | rights::WRITE | rights::WAIT,
        )
    }
    pub fn acquire_wait(&mut self, pid: ProcessId, handle: u64, needed: u64) -> Result<WaitLease> {
        let g = self.grant(pid, handle, needed)?;
        let Kind::Platform(Resource::Wait { kind, wait }) = self.object(g.object).kind else {
            return Err(Status::InvalidArgument);
        };
        self.retain(g.object);
        Ok(WaitLease {
            kind,
            wait,
            lease: Lease { object: g.object },
        })
    }
    pub fn create_thread(
        &mut self,
        pid: ProcessId,
        key: u64,
        wait: WaitId,
    ) -> Result<(u64, Lease)> {
        if key == 0 {
            return Err(Status::InvalidArgument);
        }
        let handle = self.create_resource(
            pid,
            Resource::Thread {
                key,
                owner: pid,
                wait,
                exit: None,
            },
            COPY | rights::WAIT,
        )?;
        let g = self.grant(pid, handle, 0)?;
        self.retain(g.object);
        Ok((handle, Lease { object: g.object }))
    }
    /// Publish the stable result after context retirement. The runtime must keep
    /// its lease through the subsequent scheduler completion signal: the final
    /// reference queues the associated wait object for immediate cleanup.
    /// A new handle to the live thread object `key` of `pid` (a thread
    /// opening itself: DuplicateHandle of GetCurrentThread, P18).
    pub fn open_thread(&mut self, pid: ProcessId, key: u64) -> Result<u64> {
        let object = self
            .objects
            .iter()
            .flatten()
            .find(|o| {
                matches!(
                    o.kind,
                    Kind::Platform(Resource::Thread { key: k, owner, exit: None, .. })
                        if k == key && owner == pid
                )
            })
            .map(|o| o.id)
            .ok_or(Status::NotFound)?;
        self.handle_capacity(pid, 1)?;
        Ok(self.install(pid, object, COPY | rights::WAIT))
    }
    pub fn complete_thread(&mut self, lease: &Lease, result: i64) -> Result<()> {
        let Kind::Platform(Resource::Thread { exit, .. }) = &mut self.object_mut(lease.object).kind
        else {
            return Err(Status::InvalidArgument);
        };
        if exit.is_some() {
            return Err(Status::AlreadyExists);
        }
        *exit = Some(result);
        Ok(())
    }
    pub fn acquire_thread(&mut self, pid: ProcessId, handle: u64) -> Result<PlatformLease> {
        let g = self.grant(pid, handle, rights::WAIT)?;
        let Kind::Platform(Resource::Thread {
            key,
            owner,
            wait,
            exit,
        }) = self.object(g.object).kind
        else {
            return Err(Status::InvalidArgument);
        };
        if owner != pid {
            return Err(Status::AccessDenied);
        }
        self.retain(g.object);
        Ok(PlatformLease {
            key,
            wait,
            exit,
            lease: Lease { object: g.object },
        })
    }
    pub fn thread_result(&self, lease: &Lease) -> Result<Option<i64>> {
        if let Kind::Platform(Resource::Thread { exit, .. }) = self.object(lease.object).kind {
            Ok(exit)
        } else {
            Err(Status::InvalidArgument)
        }
    }
    pub fn take_retired_wait(&mut self) -> Option<WaitId> {
        self.retired_waits
            .iter_mut()
            .find(|v| v.is_some())
            .and_then(Option::take)
    }
    pub fn active_sessions(&self) -> usize {
        self.sessions
            .iter()
            .flatten()
            .filter(|s| !s.revoked)
            .count()
    }
    pub(super) fn drop_platform(&mut self, resource: Resource) {
        if let Resource::Session { id } = resource
            && let Some(session) = self.sessions.iter_mut().flatten().find(|s| s.id == id)
        {
            // Both handles/queued grants and bound processes retain this
            // object. Orphaned publication can therefore reclaim safely;
            // a process keeps its session alive until VM/thread retirement.
            session.revoked = true;
        }
        if let Resource::Wait { wait, .. } | Resource::Thread { wait, .. } = resource {
            *self
                .retired_waits
                .iter_mut()
                .find(|v| v.is_none())
                .expect("bounded wait retirement") = Some(wait);
        }
    }
}
