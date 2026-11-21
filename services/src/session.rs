use crate::credentials::{self, Persist, PersistError, STORE_BYTES, Store};
use crate::server::{self, Handler, Response, Server};
use door_abi::{
    Handle, HandleKind, Transfer,
    platform::{Identity, StartupV2, inherited},
    rights,
};
use door_service_protocol::{
    Code,
    credential::{self, Update},
    manifest, opcode,
};

/// The protected credential directory and store (ADR 0020, contract
/// `desktop-shell-v1`): owner uid 2 (this service), integrity 2, no other
/// entry, provisioned by the storage service's system identity. Without it the
/// store is volatile (enrollment lasts for this boot) and says so. The file
/// keeps its v1 name: a v1 record is a valid store of format v2.
pub const CREDENTIAL_DIRECTORY: &str = "/state/credentials";
pub const CREDENTIAL_PATH: &str = "/state/credentials/local.v1";
/// The second account's credentials (uid 1002).
pub const SECOND_CREDENTIAL_PATH: &str = "/state/credentials/local.1002.v1";
/// The local accounts (`credential::Accounts`): absent until a second
/// account is added; the first account (1001, an administrator) needs none.
pub const ACCOUNTS_PATH: &str = "/state/credentials/accounts.v1";

/// Session-service heap: the Argon2 working memory plus slack.
pub const HEAP_BYTES: u64 = 8 * 1024 * 1024;
// The whole store is read in one storage request.
const _: () = assert!(STORE_BYTES <= door_service_protocol::CHUNK_BYTES);

struct StoragePersist {
    registry: Handle,
    storage: Option<door_service_client::Storage>,
    path: &'static str,
}
impl StoragePersist {
    fn storage(&mut self) -> Result<&mut door_service_client::Storage, PersistError> {
        if self.storage.as_ref().is_some_and(|s| s.needs_reconnect()) {
            self.storage = None;
        }
        if self.storage.is_none() {
            let storage = door_service_client::Storage::connect(self.registry)
                .map_err(|_| PersistError::Unavailable)?;
            self.storage = Some(storage);
        }
        self.storage.as_mut().ok_or(PersistError::Unavailable)
    }
}
fn missing(error: door_service_client::Error) -> bool {
    matches!(
        error,
        door_service_client::Error::Service(Code::NotFound | Code::Denied)
    )
}
impl Persist for StoragePersist {
    fn load(&mut self, out: &mut [u8; STORE_BYTES]) -> Result<Option<usize>, PersistError> {
        use door_service_client::{Disposition, READ};
        let path = self.path;
        let storage = self.storage()?;
        match storage.open(path, Disposition::OpenExisting, READ, READ) {
            Ok(file) => {
                let read = storage.info(&file).and_then(|info| {
                    let len = usize::try_from(info.bytes)
                        .ok()
                        .filter(|&len| len <= STORE_BYTES)
                        .ok_or(door_service_client::Error::Service(Code::Corrupt))?;
                    // The whole store fits one read (STORE_BYTES = CHUNK_BYTES).
                    let read = if len == 0 {
                        0
                    } else {
                        storage.read(&file, 0, &mut out[..len])?
                    };
                    Ok((read, len))
                });
                let _ = storage.close(file);
                match read {
                    Ok((read, len)) if read == len => Ok(Some(len)),
                    _ => Err(PersistError::Failed),
                }
            }
            Err(error) if missing(error) => match storage.metadata(CREDENTIAL_DIRECTORY) {
                Ok(_) => Ok(None),
                Err(error) if missing(error) => Err(PersistError::Unavailable),
                Err(_) => Err(PersistError::Failed),
            },
            Err(_) => Err(PersistError::Failed),
        }
    }
    /// An existing store (v1 or v2, or the empty file of an interrupted first
    /// enrollment) is replaced in one durable payload/length/version
    /// transaction (`replace_checked`), so a crash leaves the old or the new
    /// credentials. Only the very first store is created, then written once.
    fn save(&mut self, bytes: &[u8]) -> Result<(), PersistError> {
        use door_service_client::{Disposition, Error, READ, WRITE};
        let path = self.path;
        let storage = self.storage()?;
        match storage.open(path, Disposition::OpenExisting, READ | WRITE, 0) {
            Ok(file) => {
                let replaced = storage
                    .info(&file)
                    .and_then(|info| storage.replace_checked(&file, info, bytes));
                let _ = storage.close(file);
                replaced.map(|_| ()).map_err(|_| PersistError::Failed)
            }
            Err(Error::Service(Code::NotFound)) => {
                let file = storage
                    .open(path, Disposition::CreateNew, WRITE, 0)
                    .map_err(|_| PersistError::Failed)?;
                // One journal transaction: the store is below a 4 KiB chunk.
                let written = storage.write_all(&file, 0, bytes);
                let _ = storage.close(file);
                written.map_err(|_| PersistError::Failed)
            }
            Err(_) => Err(PersistError::Failed),
        }
    }
}

/// The logged-on session, learned when this service creates it at LOGON.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LiveSession {
    pub id: u64,
    pub epoch: u64,
    pub uid: u32,
}

/// Who may reach the credential store (ADR 0020, contract `desktop-shell-v1`
/// section 4). The trusted sign-in surface (the session-less window server)
/// may use every operation; Init may read the state; the Settings app of the
/// logged-on session (workspace manifest, that session's id and epoch, its
/// user, no capabilities) may read the state and change, add or remove a
/// credential, each of which proves a current secret first. Every other
/// process, and every process of any other or revoked session, is denied.
pub fn credential_allowed(peer: &Identity, operation: u32, live: Option<LiveSession>) -> bool {
    use door_abi::platform::manifest::{WINDOW, WORKSPACE};
    let surface = peer.manifest == WINDOW && peer.session == 0;
    let init = peer.manifest == manifest::INIT && peer.session == 0;
    let settings = peer.manifest == WORKSPACE
        && server::session_peer(*peer)
        && live.is_some_and(|live| {
            (live.id, live.epoch, live.uid) == (peer.session, peer.epoch, peer.uid)
        });
    match operation {
        credential::STATE | credential::ACCOUNTS => surface || init || settings,
        credential::ENROLL | credential::VERIFY | credential::SELECT => surface,
        credential::CHANGE | credential::ADD | credential::REMOVE => surface || settings,
        // Whether the caller's account is an administrator is checked with
        // the account book.
        credential::ACCOUNT_CREATE | credential::ACCOUNT_TYPE | credential::ACCOUNT_DELETE => {
            settings
        }
        _ => false,
    }
}

const fn kind_name(kind: credential::Kind) -> &'static [u8] {
    match kind {
        credential::Kind::Pin => b"pin",
        credential::Kind::Password => b"password",
    }
}

/// The local accounts, persisted in `ACCOUNTS_PATH`.
struct AccountBook {
    persist: StoragePersist,
    list: Option<credential::Accounts>,
}
impl Session<'_> {
    /// Loads the account book while Init can still answer this service's
    /// storage lookup. Only with a storage connection: without one the book
    /// stays unloaded (no account is taken for an administrator by default)
    /// and the first request that needs it loads it.
    fn preload_accounts(&mut self) {
        if self.accounts.persist.storage().is_ok() {
            let _ = self.accounts.get();
        }
    }
}
impl AccountBook {
    /// The first account alone until a second one is added.
    fn initial() -> credential::Accounts {
        let mut accounts = credential::Accounts::default();
        accounts.list[0] = credential::Account::new(credentials::LOCAL_UID, "User", true).ok();
        accounts
    }
    fn get(&mut self) -> Result<credential::Accounts, Code> {
        if let Some(list) = self.list {
            return Ok(list);
        }
        let mut bytes = [0; STORE_BYTES];
        let list = match self.persist.load(&mut bytes) {
            Ok(Some(n)) => credential::Accounts::decode(&bytes[..n])?,
            Ok(None) | Err(PersistError::Unavailable) => Self::initial(),
            Err(PersistError::Failed) => return Err(Code::Corrupt),
        };
        // The first account always exists.
        if list.get(credentials::LOCAL_UID).is_none() {
            return Err(Code::Corrupt);
        }
        self.list = Some(list);
        Ok(list)
    }
    fn put(&mut self, list: credential::Accounts) -> Result<(), Code> {
        let mut bytes = [0; credential::Accounts::MAX_BYTES];
        let n = list.encode(&mut bytes);
        self.persist.save(&bytes[..n]).map_err(|_| Code::Io)?;
        self.list = Some(list);
        Ok(())
    }
}
/// A LOGON Init asked for and this service refused: the reason, on the
/// serial log (a refused logon stops the boot).
fn logon_refused(stage: &str, code: Code) {
    let mut line = [0u8; 96];
    let mut at = 0;
    for part in [
        b"DOOR_P08 SESSION LOGON refused stage=".as_slice(),
        stage.as_bytes(),
        b" code=",
    ] {
        line[at..at + part.len()].copy_from_slice(part);
        at += part.len();
    }
    let digits = code as u32;
    let mut text = [0u8; 10];
    let mut n = 0;
    let mut v = digits;
    loop {
        text[n] = b'0' + (v % 10) as u8;
        n += 1;
        v /= 10;
        if v == 0 {
            break;
        }
    }
    for i in (0..n).rev() {
        line[at] = text[i];
        at += 1;
    }
    line[at] = b'\n';
    let _ = door_sys::write(&line[..=at]);
}
/// `credential::ACCOUNT_UIDS` index.
fn slot(uid: u32) -> Option<usize> {
    credential::ACCOUNT_UIDS.iter().position(|&u| u == uid)
}

struct Session<'a> {
    authority: Handle,
    /// Per `credential::ACCOUNT_UIDS`.
    stores: [Store<StoragePersist>; 2],
    accounts: AccountBook,
    /// The account the sign-in surface is working with (`SELECT`).
    selected: usize,
    memory: &'a mut [argon2::Block],
    live: Option<LiveSession>,
}
impl Session<'_> {
    fn entropy(out: &mut [u8]) -> Result<(), Code> {
        match door_sys::entropy_read(out) {
            Ok(n) if n == out.len() => Ok(()),
            _ => Err(Code::Resources),
        }
    }
    /// ACCOUNTS, SELECT and the administrator's account operations.
    fn account_request(&mut self, peer: Identity, operation: u32, body: &[u8]) -> Response {
        let list = match self.accounts.get() {
            Ok(list) => list,
            Err(code) => return Response::error(code),
        };
        match operation {
            credential::ACCOUNTS if body.is_empty() => {
                let mut out = [0; credential::Accounts::MAX_BYTES];
                let n = list.encode(&mut out);
                Response::bytes(&out[..n])
            }
            credential::SELECT => {
                let Ok(uid) = <[u8; 4]>::try_from(body).map(u32::from_le_bytes) else {
                    return Response::error(Code::InvalidRequest);
                };
                match (list.get(uid), slot(uid)) {
                    (Some(_), Some(index)) => {
                        self.selected = index;
                        Response::empty()
                    }
                    _ => Response::error(Code::NotFound),
                }
            }
            _ => {
                // Settings of an administrator account only.
                if !list.get(peer.uid).is_some_and(|a| a.administrator) {
                    return Response::error(Code::Denied);
                }
                match self.administer(peer, list, operation, body) {
                    Ok(reply) => reply,
                    Err(code) => Response::error(code),
                }
            }
        }
    }
    fn administer(
        &mut self,
        peer: Identity,
        mut list: credential::Accounts,
        operation: u32,
        body: &[u8],
    ) -> Result<Response, Code> {
        match operation {
            credential::ACCOUNT_CREATE => {
                let (&n, rest) = body.split_first().ok_or(Code::InvalidRequest)?;
                let name = rest
                    .get(..usize::from(n))
                    .and_then(|b| core::str::from_utf8(b).ok())
                    .ok_or(Code::InvalidRequest)?;
                let rest = &rest[usize::from(n)..];
                let (&p, password) = rest.split_first().ok_or(Code::InvalidRequest)?;
                if password.len() != usize::from(p) {
                    return Err(Code::InvalidRequest);
                }
                credential::validate(credential::Kind::Password, password)?;
                if list.iter().any(|a| a.name().eq_ignore_ascii_case(name)) {
                    return Err(Code::Exists);
                }
                let (index, uid) = credential::ACCOUNT_UIDS
                    .iter()
                    .enumerate()
                    .find(|(_, uid)| list.get(**uid).is_none())
                    .map(|(i, uid)| (i, *uid))
                    .ok_or(Code::Resources)?;
                let account = credential::Account::new(uid, name, false)?;
                // A deleted account's credentials never carry over.
                self.stores[index].clear()?;
                self.stores[index].enroll(
                    credential::Kind::Password,
                    password,
                    &mut Self::entropy,
                    self.memory,
                )?;
                let free = list
                    .list
                    .iter()
                    .position(Option::is_none)
                    .ok_or(Code::Resources)?;
                list.list[free] = Some(account);
                self.accounts.put(list)?;
                let _ = door_sys::write(b"DOOR_P16 ACCOUNT created standard=1\n");
                Ok(Response::bytes(&uid.to_le_bytes()))
            }
            credential::ACCOUNT_TYPE => {
                if body.len() != 5 || body[4] > 1 {
                    return Err(Code::InvalidRequest);
                }
                let uid = u32::from_le_bytes(body[..4].try_into().unwrap());
                let administrator = body[4] == 1;
                let at = list
                    .list
                    .iter()
                    .position(|a| a.is_some_and(|a| a.uid == uid))
                    .ok_or(Code::NotFound)?;
                let mut account = list.list[at].unwrap();
                if account.administrator && !administrator && list.administrators() == 1 {
                    // Windows keeps at least one administrator.
                    return Err(Code::Denied);
                }
                account.administrator = administrator;
                list.list[at] = Some(account);
                self.accounts.put(list)?;
                let _ = door_sys::write(if administrator {
                    b"DOOR_P16 ACCOUNT type=administrator\n" as &[u8]
                } else {
                    b"DOOR_P16 ACCOUNT type=standard\n"
                });
                Ok(Response::empty())
            }
            credential::ACCOUNT_DELETE => {
                let uid = <[u8; 4]>::try_from(body)
                    .map(u32::from_le_bytes)
                    .map_err(|_| Code::InvalidRequest)?;
                if uid == peer.uid || self.live.is_some_and(|l| l.uid == uid) {
                    return Err(Code::Denied);
                }
                let at = list
                    .list
                    .iter()
                    .position(|a| a.is_some_and(|a| a.uid == uid))
                    .ok_or(Code::NotFound)?;
                if list.list[at].is_some_and(|a| a.administrator) && list.administrators() == 1 {
                    return Err(Code::Denied);
                }
                let index = slot(uid).ok_or(Code::NotFound)?;
                self.stores[index].clear()?;
                list.list[at] = None;
                self.accounts.put(list)?;
                if self.selected == index {
                    self.selected = 0;
                }
                let _ = door_sys::write(b"DOOR_P16 ACCOUNT deleted\n");
                Ok(Response::empty())
            }
            _ => Err(Code::InvalidRequest),
        }
    }
    fn credential_request(&mut self, peer: Identity, operation: u32, body: &[u8]) -> Response {
        if !credential_allowed(&peer, operation, self.live) {
            return Response::error(Code::Denied);
        }
        if (credential::SELECT..=credential::ACCOUNT_DELETE).contains(&operation) {
            return self.account_request(peer, operation, body);
        }
        // The surface acts on the account it selected; Init and Settings on
        // the session's own user.
        let surface = peer.manifest == door_abi::platform::manifest::WINDOW && peer.session == 0;
        let index = if surface {
            self.selected
        } else {
            match slot(if peer.session == 0 {
                self.live.map_or(credentials::LOCAL_UID, |l| l.uid)
            } else {
                peer.uid
            }) {
                Some(index) => index,
                None => return Response::error(Code::Denied),
            }
        };
        let store = &mut self.stores[index];
        let now = door_sys::time().unwrap_or(0);
        match operation {
            credential::STATE if body.is_empty() => match store.state(now) {
                Ok(state) => Response::bytes(&state.encode()),
                Err(code) => Response::error(code),
            },
            credential::ENROLL => {
                let Ok((kind, secret)) = credential::decode_secret(body) else {
                    return Response::error(Code::InvalidRequest);
                };
                // Only the first account enrolls at first run; a second
                // account gets its password from the administrator.
                if index != 0 {
                    return Response::error(Code::Denied);
                }
                match store.enroll(kind, secret, &mut Self::entropy, self.memory) {
                    Ok(state) => {
                        let _ = door_sys::write(if state.persisted {
                            b"DOOR_P14 CREDENTIAL enrolled persisted=1\n" as &[u8]
                        } else {
                            b"DOOR_P14 CREDENTIAL enrolled persisted=0\n"
                        });
                        Response::bytes(&state.encode())
                    }
                    Err(code) => Response::error(code),
                }
            }
            credential::VERIFY => {
                let Ok((kind, secret)) = credential::decode_secret(body) else {
                    // Malformed or policy-violating input counts as a wrong
                    // secret inside the store; reject framing errors here.
                    return Response::error(Code::InvalidRequest);
                };
                match store.verify(kind, secret, now, self.memory) {
                    Ok(verdict) => Response::bytes(&verdict.encode()),
                    Err(code) => Response::error(code),
                }
            }
            _ => {
                let Some(update) = Update::from_opcode(operation) else {
                    return Response::error(Code::InvalidRequest);
                };
                let Ok((kind, current, new)) = credential::decode_update(update, body) else {
                    return Response::error(Code::InvalidRequest);
                };
                let result = store.update(
                    update,
                    kind,
                    current,
                    new,
                    now,
                    &mut || door_sys::time().unwrap_or(0),
                    &mut Self::entropy,
                    self.memory,
                );
                match result {
                    Ok(outcome) => {
                        if outcome.verdict == credential::Verdict::Accepted {
                            Self::updated(update, kind, outcome.state.persisted);
                        }
                        Response::bytes(&outcome.encode())
                    }
                    Err(code) => Response::error(code),
                }
            }
        }
    }
    /// `DOOR_P14 CREDENTIAL updated op=change|add|remove kind=pin|password
    /// persisted=0|1` (never secret material).
    fn updated(update: Update, kind: credential::Kind, persisted: bool) {
        let op: &[u8] = match update {
            Update::Change => b"change",
            Update::Add => b"add",
            Update::Remove => b"remove",
        };
        let mut line = [0u8; 80];
        let mut at = 0;
        for part in [
            b"DOOR_P14 CREDENTIAL updated op=" as &[u8],
            op,
            b" kind=",
            kind_name(kind),
            if persisted {
                b" persisted=1\n"
            } else {
                b" persisted=0\n"
            },
        ] {
            line[at..at + part.len()].copy_from_slice(part);
            at += part.len();
        }
        let _ = door_sys::write(&line[..at]);
    }
}
impl Handler for Session<'_> {
    fn request(
        &mut self,
        _channel: Handle,
        peer: Identity,
        operation: u32,
        body: &[u8],
        transfers: &[Transfer],
    ) -> Response {
        if (credential::STATE..=credential::ACCOUNT_DELETE).contains(&operation) {
            if !transfers.is_empty() {
                return Response::error(Code::Denied);
            }
            return self.credential_request(peer, operation, body);
        }
        if peer.manifest != manifest::INIT || !body.is_empty() || transfers.len() != 1 {
            if operation == opcode::LOGON {
                logon_refused("request", Code::Denied);
            }
            return Response::error(Code::Denied);
        }
        let capability = transfers[0].handle;
        match operation {
            opcode::LOGON => {
                if let Err(error) =
                    server::transferred_capability(capability, HandleKind::LogonPermit)
                {
                    logon_refused("permit", server::status(error));
                    return Response::error(server::status(error));
                }
                // The account book says which accounts are administrators:
                // their applications get the filtered token.
                let administrators = match self.accounts.get() {
                    Ok(list) => door_abi::platform::ACCOUNTS
                        .iter()
                        .enumerate()
                        .filter(|(_, uid)| list.get(**uid).is_some_and(|a| a.administrator))
                        .fold(0u64, |mask, (i, _)| mask | 1 << i),
                    Err(code) => {
                        logon_refused("account book", code);
                        return Response::error(code);
                    }
                };
                match door_sys::session_create(self.authority, capability, administrators) {
                    Ok(session) => {
                        // Its Settings app may manage the credentials (the
                        // service account's session has no Settings and
                        // leaves the signed-in user's in place).
                        let created = door_sys::session_info(session)
                            .ok()
                            .map(|(id, epoch, uid)| LiveSession { id, epoch, uid });
                        if created.is_some_and(|s| s.uid != door_abi::platform::SERVICE_ACCOUNT) {
                            self.live = created;
                        }
                        Response::handle(
                            session,
                            rights::QUERY
                                | rights::TRANSFER
                                | rights::DUPLICATE
                                | rights::READ
                                | rights::WRITE,
                            true,
                        )
                    }
                    Err(error) => {
                        logon_refused("session", server::status(error));
                        Response::error(server::status(error))
                    }
                }
            }
            opcode::LOGOFF => {
                if let Err(error) = server::transferred_capability(capability, HandleKind::Session)
                {
                    return Response::error(server::status(error));
                }
                let id = door_sys::session_info(capability).ok().map(|info| info.0);
                match door_sys::session_revoke(self.authority, capability) {
                    Ok(()) => {
                        // Forget the session unless another one was revoked.
                        if id.is_none() || id == self.live.map(|live| live.id) {
                            self.live = None;
                        }
                        Response::empty()
                    }
                    Err(error) => Response::error(server::status(error)),
                }
            }
            _ => Response::error(Code::Unsupported),
        }
    }
}
/// `memory` is the Argon2 working set for `credentials::PARAMS`, allocated
/// once by the session binary from its own heap.
pub fn run(startup: StartupV2, memory: &mut [argon2::Block]) -> i64 {
    if credentials::PARAMS
        .blocks()
        .is_ok_and(|blocks| memory.len() < blocks)
    {
        return -132;
    }
    let server = match Server::new(&startup) {
        Ok(server) => server,
        Err(_) => return -130,
    };
    let mut session = Session {
        authority: startup.base.inherited[inherited::AUTHORITY].handle,
        stores: [
            Store::new(
                StoragePersist {
                    registry: startup.base.inherited[inherited::REGISTRY].handle,
                    storage: None,
                    path: CREDENTIAL_PATH,
                },
                credentials::PARAMS,
            ),
            Store::for_account(
                StoragePersist {
                    registry: startup.base.inherited[inherited::REGISTRY].handle,
                    storage: None,
                    path: SECOND_CREDENTIAL_PATH,
                },
                credentials::PARAMS,
                credential::ACCOUNT_UIDS[1],
            ),
        ],
        accounts: AccountBook {
            persist: StoragePersist {
                registry: startup.base.inherited[inherited::REGISTRY].handle,
                storage: None,
                path: ACCOUNTS_PATH,
            },
            list: None,
        },
        selected: 0,
        memory,
        live: None,
    };
    // The account book before READY: Init's LOGON (a synchronous call)
    // must not wait on this service resolving storage through Init. The
    // storage service is ready before this one starts (its dependency).
    session.preload_accounts();
    if server.ready(manifest::SESSION).is_err() {
        return -131;
    }
    server::serve(server, session)
}

#[cfg(test)]
mod tests {
    use super::*;
    use door_abi::platform::manifest::{SHELL, WINDOW, WORKSPACE};

    fn peer(manifest: u64, session: u64) -> Identity {
        Identity {
            pid: 40,
            uid: if session == 0 {
                2
            } else {
                credentials::LOCAL_UID
            },
            integrity: if session == 0 { 2 } else { 1 },
            groups: 0,
            session,
            epoch: if session == 0 { 0 } else { 9 },
            manifest,
            instance: 1,
            capabilities: 0,
        }
    }
    const ALL: [u32; 6] = [
        credential::STATE,
        credential::ENROLL,
        credential::VERIFY,
        credential::CHANGE,
        credential::ADD,
        credential::REMOVE,
    ];

    #[test]
    fn credential_callers_are_the_surface_init_and_the_live_settings_app() {
        let live = Some(LiveSession {
            id: 5,
            epoch: 9,
            uid: credentials::LOCAL_UID,
        });
        let allowed = |peer: Identity, live| -> [bool; 6] {
            ALL.map(|operation| credential_allowed(&peer, operation, live))
        };
        // The sign-in surface: everything, with or without a session.
        assert_eq!(allowed(peer(WINDOW, 0), None), [true; 6]);
        assert_eq!(allowed(peer(WINDOW, 0), live), [true; 6]);
        // Init: the state only.
        assert_eq!(
            allowed(peer(manifest::INIT, 0), live),
            [true, false, false, false, false, false]
        );
        // The live session's Settings: state and the three updates, never
        // enrollment or a bare verify (no password oracle).
        let settings = peer(WORKSPACE, 5);
        assert_eq!(
            allowed(settings, live),
            [true, false, false, true, true, true]
        );
        // ... and only while that session is the logged-on one.
        assert_eq!(allowed(settings, None), [false; 6]);
        for (id, epoch, uid) in [(6, 9, credentials::LOCAL_UID), (5, 8, 1001), (5, 9, 1002)] {
            let other = Some(LiveSession { id, epoch, uid });
            assert_eq!(allowed(settings, other), [false; 6], "{id} {epoch} {uid}");
        }
        // A capability-holding or system process with the same manifest is
        // no session application.
        let mut privileged = settings;
        privileged.capabilities = 1;
        assert_eq!(allowed(privileged, live), [false; 6]);
        // A window server inside a session is not the sign-in surface.
        assert_eq!(allowed(peer(WINDOW, 5), live), [false; 6]);
        // Other session processes and services are denied.
        for manifest in [SHELL, door_abi::platform::manifest::DESKTOP_SHELL, 99] {
            assert_eq!(allowed(peer(manifest, 5), live), [false; 6]);
        }
        assert_eq!(allowed(peer(manifest::STORAGE, 0), live), [false; 6]);
        assert!(!credential_allowed(
            &peer(WINDOW, 0),
            credential::ACCOUNT_DELETE + 1,
            live
        ));
        // Accounts: everyone allowed above may list them; only the surface
        // picks whose credentials it checks; only the live Settings
        // administers (and the service checks it is an administrator's).
        let accounts = [
            credential::ACCOUNTS,
            credential::SELECT,
            credential::ACCOUNT_CREATE,
            credential::ACCOUNT_TYPE,
            credential::ACCOUNT_DELETE,
        ];
        let account_ops =
            |peer: Identity| accounts.map(|operation| credential_allowed(&peer, operation, live));
        assert_eq!(
            account_ops(peer(WINDOW, 0)),
            [true, true, false, false, false]
        );
        assert_eq!(
            account_ops(peer(manifest::INIT, 0)),
            [true, false, false, false, false]
        );
        assert_eq!(account_ops(settings), [true, false, true, true, true]);
        assert_eq!(account_ops(peer(SHELL, 5)), [false; 5]);
    }
    #[test]
    fn the_second_account_has_its_own_name_rules_and_wire_form() {
        use credential::{Account, Accounts, valid_name};
        assert!(valid_name("Guest Two"));
        assert!(!valid_name(""));
        assert!(!valid_name("..."));
        assert!(!valid_name("a/b"));
        assert!(!valid_name("123456789012345678901"));
        let mut list = AccountBook::initial();
        assert_eq!(list.administrators(), 1);
        list.list[1] = Account::new(1002, "Guest Two", false).ok();
        let mut bytes = [0; Accounts::MAX_BYTES];
        let n = list.encode(&mut bytes);
        assert_eq!(Accounts::decode(&bytes[..n]), Ok(list));
        assert_eq!(list.get(1002).map(|a| a.name()), Some("Guest Two"));
        assert!(Account::new(1003, "x", false).is_err());
        for len in 0..n {
            assert!(Accounts::decode(&bytes[..len]).is_err(), "{len}");
        }
        assert_eq!(slot(1001), Some(0));
        assert_eq!(slot(1002), Some(1));
        assert_eq!(slot(1000), None);
    }
}
