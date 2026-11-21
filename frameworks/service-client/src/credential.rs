//! Client of the session service's credential store (P14-LOGON): the window
//! server's sign-in surface and the session's Settings app. Requests are begun
//! and polled so neither display loop ever waits for Argon2.
use crate::rpc::{Connection, Error, Result};
use door_abi::Handle;
use door_service_protocol::{
    credential::{self, Kind, Outcome, SECRET_MAX, State, UPDATE_MAX, Update, Verdict},
    manifest,
};

/// A completed credential request.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Reply {
    State(State),
    Enrolled(State),
    Verdict(Verdict),
    /// `begin_change`, `begin_add` or `begin_remove` finished: the verdict on
    /// the current secret (`Accepted` means the update was applied) and the
    /// state afterwards.
    Updated(Outcome),
}

pub struct Credentials {
    connection: Connection,
    pending: Option<u32>,
}
impl Credentials {
    pub fn connect(registry: Handle) -> Result<Self> {
        Ok(Self {
            connection: crate::connect(registry, manifest::SESSION)?,
            pending: None,
        })
    }
    /// Connect through a port already resolved (`Init::begin_resolve`); the
    /// caller keeps and closes `port`.
    pub fn connect_port(port: Handle) -> Result<Self> {
        Ok(Self {
            connection: Connection::connect(port, manifest::SESSION)?,
            pending: None,
        })
    }
    pub fn needs_reconnect(&self) -> bool {
        self.connection.needs_reconnect()
    }
    pub fn busy(&self) -> bool {
        self.pending.is_some()
    }
    pub fn begin_state(&mut self, now: u64) -> Result<()> {
        self.begin(credential::STATE, &[], now)
    }
    pub fn begin_enroll(&mut self, kind: Kind, secret: &[u8], now: u64) -> Result<()> {
        let mut body = [0u8; 2 + SECRET_MAX];
        let len = credential::encode_secret(kind, secret, &mut body).map_err(Error::Service)?;
        let result = self.begin(credential::ENROLL, &body[..len], now);
        wipe(&mut body);
        result
    }
    /// Malformed or policy-violating input never leaves this process; the
    /// caller reports it as a wrong secret without contacting the service.
    pub fn begin_verify(&mut self, kind: Kind, secret: &[u8], now: u64) -> Result<()> {
        let mut body = [0u8; 2 + SECRET_MAX];
        let len = credential::encode_secret(kind, secret, &mut body).map_err(Error::Service)?;
        let result = self.begin(credential::VERIFY, &body[..len], now);
        wipe(&mut body);
        result
    }
    /// Change the secret of the enrolled `kind` from `current` to `new` (both
    /// of `kind`). A secret that fails its policy is refused here with
    /// `Error::Service(Code::InvalidRequest)` and never sent.
    pub fn begin_change(&mut self, kind: Kind, current: &[u8], new: &[u8], now: u64) -> Result<()> {
        self.begin_update(Update::Change, kind, current, new, now)
    }
    /// Add `kind`, which is not enrolled yet, with the secret `new`, proved
    /// with `current`, the secret of the other (enrolled) kind: a PIN with
    /// the password, or a password with the PIN.
    pub fn begin_add(&mut self, kind: Kind, current: &[u8], new: &[u8], now: u64) -> Result<()> {
        self.begin_update(Update::Add, kind, current, new, now)
    }
    /// Remove `kind`, proved with `current`, the secret of the other kind,
    /// which must be enrolled and stays enrolled.
    pub fn begin_remove(&mut self, kind: Kind, current: &[u8], now: u64) -> Result<()> {
        self.begin_update(Update::Remove, kind, current, &[], now)
    }
    fn begin_update(
        &mut self,
        update: Update,
        kind: Kind,
        current: &[u8],
        new: &[u8],
        now: u64,
    ) -> Result<()> {
        let mut body = [0u8; UPDATE_MAX];
        let len = credential::encode_update(update, kind, current, new, &mut body)
            .map_err(Error::Service)?;
        let result = self.begin(update.opcode(), &body[..len], now);
        wipe(&mut body);
        result
    }
    fn begin(&mut self, opcode: u32, body: &[u8], now: u64) -> Result<()> {
        self.connection.begin(opcode, body, now)?;
        self.pending = Some(opcode);
        Ok(())
    }
    /// The local accounts (blocking: a small read, no Argon2).
    pub fn accounts(&mut self) -> Result<credential::Accounts> {
        if self.pending.is_some() {
            return Err(Error::Service(door_service_protocol::Code::WouldBlock));
        }
        let reply = self.connection.call(credential::ACCOUNTS, &[], &[])?;
        credential::Accounts::decode(reply.data()?).map_err(|_| Error::Malformed)
    }
    /// The sign-in surface's account (`credential::SELECT`).
    pub fn select(&mut self, uid: u32) -> Result<()> {
        if self.pending.is_some() {
            return Err(Error::Service(door_service_protocol::Code::WouldBlock));
        }
        self.connection
            .call(credential::SELECT, &uid.to_le_bytes(), &[])?
            .empty()
    }
    /// An administrator's Settings: a new standard account with a password
    /// (blocking: one Argon2 derivation, well inside the reply deadline).
    pub fn create_account(&mut self, name: &str, password: &[u8]) -> Result<u32> {
        if !credential::valid_name(name) || password.len() > SECRET_MAX {
            return Err(Error::Service(door_service_protocol::Code::InvalidRequest));
        }
        let mut body = [0u8; 2 + credential::NAME_MAX * 4 + SECRET_MAX];
        body[0] = name.len() as u8;
        body[1..1 + name.len()].copy_from_slice(name.as_bytes());
        let at = 1 + name.len();
        body[at] = password.len() as u8;
        body[at + 1..at + 1 + password.len()].copy_from_slice(password);
        let len = at + 1 + password.len();
        let reply = self
            .connection
            .call(credential::ACCOUNT_CREATE, &body[..len], &[]);
        wipe(&mut body);
        let data = reply?;
        let data = data.data()?;
        <[u8; 4]>::try_from(data)
            .map(u32::from_le_bytes)
            .map_err(|_| Error::Malformed)
    }
    pub fn set_account_type(&mut self, uid: u32, administrator: bool) -> Result<()> {
        let mut body = [0u8; 5];
        body[..4].copy_from_slice(&uid.to_le_bytes());
        body[4] = u8::from(administrator);
        self.connection
            .call(credential::ACCOUNT_TYPE, &body, &[])?
            .empty()
    }
    pub fn delete_account(&mut self, uid: u32) -> Result<()> {
        self.connection
            .call(credential::ACCOUNT_DELETE, &uid.to_le_bytes(), &[])?
            .empty()
    }
    pub fn poll(&mut self, now: u64) -> Result<Option<Reply>> {
        let Some(opcode) = self.pending else {
            return Err(Error::Malformed);
        };
        let reply = match self.connection.poll(now) {
            Ok(None) => return Ok(None),
            other => {
                self.pending = None;
                other?
            }
        };
        let Some(reply) = reply else {
            return Ok(None);
        };
        decode(opcode, reply.data()?)
            .map(Some)
            .map_err(|_| Error::Malformed)
    }
}
fn decode(opcode: u32, data: &[u8]) -> core::result::Result<Reply, door_service_protocol::Code> {
    match opcode {
        credential::STATE => State::decode(data).map(Reply::State),
        credential::ENROLL => State::decode(data).map(Reply::Enrolled),
        credential::CHANGE | credential::ADD | credential::REMOVE => {
            Outcome::decode(data).map(Reply::Updated)
        }
        _ => Verdict::decode(data).map(Reply::Verdict),
    }
}
/// The request buffer held a secret; clear it in a way the optimizer keeps.
fn wipe(bytes: &mut [u8]) {
    bytes.fill(0);
    core::hint::black_box(bytes);
}

#[cfg(test)]
mod tests {
    use super::*;
    use door_service_protocol::Code;

    #[test]
    fn replies_decode_per_operation() {
        let state = State {
            enrolled: Some(Kind::Pin),
            both: true,
            persisted: true,
            failures: 0,
            retry_after: 0,
        };
        let outcome = Outcome {
            verdict: Verdict::Accepted,
            state,
        };
        assert_eq!(
            decode(credential::STATE, &state.encode()),
            Ok(Reply::State(state))
        );
        assert_eq!(
            decode(credential::ENROLL, &state.encode()),
            Ok(Reply::Enrolled(state))
        );
        for opcode in [credential::CHANGE, credential::ADD, credential::REMOVE] {
            assert_eq!(
                decode(opcode, &outcome.encode()),
                Ok(Reply::Updated(outcome))
            );
            // A bare verdict is not an update reply.
            assert_eq!(
                decode(opcode, &Verdict::Accepted.encode()),
                Err(Code::InvalidRequest)
            );
        }
        assert_eq!(
            decode(credential::VERIFY, &Verdict::Accepted.encode()),
            Ok(Reply::Verdict(Verdict::Accepted))
        );
        let mut secret = *b"2468";
        wipe(&mut secret);
        assert_eq!(secret, [0; 4]);
    }
}
