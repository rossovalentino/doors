//! Trusted sign-in credential protocol (P14-LOGON; `specs/contracts/desktop-shell-v1.md`).
//!
//! The session service owns the credential records of the local account (an
//! Argon2id hash with a kernel-entropy salt per kind: a password, a PIN, or
//! both). Only the current window server, which draws the trusted sign-in and
//! lock surfaces, may enroll or verify; Init may query the state; the logged-on
//! session's Settings app may query the state and change, add or remove a
//! credential after proving a current one. A secret travels once per request,
//! is never stored or echoed, and no reply reveals anything beyond
//! accept/reject/throttle. Every other session process is denied.
use crate::Code;

/// Query `State` (no body). Window server, Init or the session's Settings.
pub const STATE: u32 = 0x4010;
/// First-run enrollment: `encode_secret` body. Refused once a record exists.
pub const ENROLL: u32 = 0x4011;
/// Check a secret: `encode_secret` body, `Verdict` reply.
pub const VERIFY: u32 = 0x4012;
/// Replace the secret of an enrolled kind, proved with its current secret:
/// `encode_update` body, `Outcome` reply.
pub const CHANGE: u32 = 0x4013;
/// Add the kind that is not enrolled yet, proved with the secret of the other
/// (enrolled) kind: `encode_update` body, `Outcome` reply.
pub const ADD: u32 = 0x4014;
/// Remove an enrolled kind, proved with the secret of the other kind, which
/// stays enrolled (the account never ends up without a credential):
/// `encode_update` body without a new secret, `Outcome` reply.
pub const REMOVE: u32 = 0x4015;

pub const SECRET_MAX: usize = 64;
pub const PASSWORD_MIN: usize = 4;
pub const PIN_MIN: usize = 4;
pub const PIN_MAX: usize = 16;
/// Failed attempts before throttling starts, and the first delay in
/// monotonic ticks (10 ms): 30 s, doubling per further failure up to 5 min.
pub const FREE_ATTEMPTS: u32 = 5;
pub const FIRST_DELAY_TICKS: u64 = 3_000;
pub const MAX_DELAY_TICKS: u64 = 30_000;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum Kind {
    Password = 1,
    Pin = 2,
}
impl Kind {
    pub const fn from_u8(value: u8) -> Option<Self> {
        match value {
            1 => Some(Self::Password),
            2 => Some(Self::Pin),
            _ => None,
        }
    }
    /// Whether `ch` may be typed into a box of this kind.
    pub fn accepts(self, ch: char) -> bool {
        match self {
            Self::Pin => ch.is_ascii_digit(),
            Self::Password => !ch.is_control(),
        }
    }
    pub const fn max_len(self) -> usize {
        match self {
            Self::Pin => PIN_MAX,
            Self::Password => SECRET_MAX,
        }
    }
    /// The other kind (a password for a PIN and the reverse).
    pub const fn other(self) -> Self {
        match self {
            Self::Pin => Self::Password,
            Self::Password => Self::Pin,
        }
    }
    const fn bit(self) -> u8 {
        1 << (self as u8 - 1)
    }
}

/// Policy shared by the sign-in surface and the service (which re-checks).
pub fn validate(kind: Kind, secret: &[u8]) -> Result<(), Code> {
    let Ok(text) = core::str::from_utf8(secret) else {
        return Err(Code::InvalidRequest);
    };
    let ok = match kind {
        Kind::Password => {
            (PASSWORD_MIN..=SECRET_MAX).contains(&secret.len())
                && text.chars().all(|ch| kind.accepts(ch))
        }
        Kind::Pin => {
            (PIN_MIN..=PIN_MAX).contains(&secret.len()) && secret.iter().all(u8::is_ascii_digit)
        }
    };
    if ok {
        Ok(())
    } else {
        Err(Code::InvalidRequest)
    }
}

/// `[kind][length][secret bytes]`.
pub fn encode_secret(kind: Kind, secret: &[u8], out: &mut [u8]) -> Result<usize, Code> {
    validate(kind, secret)?;
    let len = 2 + secret.len();
    if out.len() < len {
        return Err(Code::InvalidRequest);
    }
    out[0] = kind as u8;
    out[1] = secret.len() as u8;
    out[2..len].copy_from_slice(secret);
    Ok(len)
}
pub fn decode_secret(body: &[u8]) -> Result<(Kind, &[u8]), Code> {
    let (&[kind, len], secret) = body.split_first_chunk::<2>().ok_or(Code::InvalidRequest)?;
    let kind = Kind::from_u8(kind).ok_or(Code::InvalidRequest)?;
    if usize::from(len) != secret.len() {
        return Err(Code::InvalidRequest);
    }
    validate(kind, secret)?;
    Ok((kind, secret))
}

/// A credential update (`CHANGE`, `ADD`, `REMOVE`).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Update {
    Change,
    Add,
    Remove,
}
impl Update {
    pub const fn opcode(self) -> u32 {
        match self {
            Self::Change => CHANGE,
            Self::Add => ADD,
            Self::Remove => REMOVE,
        }
    }
    pub const fn from_opcode(opcode: u32) -> Option<Self> {
        match opcode {
            CHANGE => Some(Self::Change),
            ADD => Some(Self::Add),
            REMOVE => Some(Self::Remove),
            _ => None,
        }
    }
    /// The kind of the current secret that proves the caller when the update
    /// targets `kind`: the same kind for `CHANGE`, the other one otherwise.
    pub const fn proof(self, kind: Kind) -> Kind {
        match self {
            Self::Change => kind,
            Self::Add | Self::Remove => kind.other(),
        }
    }
    /// Whether the update carries a new secret (of the target kind).
    pub const fn has_new(self) -> bool {
        !matches!(self, Self::Remove)
    }
}
/// Largest `encode_update` body.
pub const UPDATE_MAX: usize = 3 + 2 * SECRET_MAX;
// Every update body fits one request.
const _: () = assert!(UPDATE_MAX <= crate::BODY_BYTES);
/// `[kind][current length][new length][current secret][new secret]`: `kind`
/// is the kind the update targets, the current secret is of
/// `update.proof(kind)` and the new one (absent for `REMOVE`) of `kind`. Both
/// must satisfy their policy; nothing that fails it is ever sent.
pub fn encode_update(
    update: Update,
    kind: Kind,
    current: &[u8],
    new: &[u8],
    out: &mut [u8],
) -> Result<usize, Code> {
    validate(update.proof(kind), current)?;
    if update.has_new() {
        validate(kind, new)?;
    } else if !new.is_empty() {
        return Err(Code::InvalidRequest);
    }
    let len = 3 + current.len() + new.len();
    if out.len() < len {
        return Err(Code::InvalidRequest);
    }
    out[0] = kind as u8;
    out[1] = current.len() as u8;
    out[2] = new.len() as u8;
    out[3..3 + current.len()].copy_from_slice(current);
    out[3 + current.len()..len].copy_from_slice(new);
    Ok(len)
}
/// The target kind, the current secret and the new secret (empty for
/// `REMOVE`) of an `encode_update` body.
pub fn decode_update(update: Update, body: &[u8]) -> Result<(Kind, &[u8], &[u8]), Code> {
    let (&[kind, current_len, new_len], rest) =
        body.split_first_chunk::<3>().ok_or(Code::InvalidRequest)?;
    let kind = Kind::from_u8(kind).ok_or(Code::InvalidRequest)?;
    let (current_len, new_len) = (usize::from(current_len), usize::from(new_len));
    if current_len + new_len != rest.len() {
        return Err(Code::InvalidRequest);
    }
    let (current, new) = rest.split_at(current_len);
    validate(update.proof(kind), current)?;
    if update.has_new() {
        validate(kind, new)?;
    } else if !new.is_empty() {
        return Err(Code::InvalidRequest);
    }
    Ok((kind, current, new))
}

/// `STATE` reply.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct State {
    /// The kind the sign-in surface asks for: the PIN whenever one is
    /// enrolled (as Windows signs in with Windows Hello first), otherwise the
    /// password; `None` before enrollment.
    pub enrolled: Option<Kind>,
    /// A password and a PIN are both enrolled (`enrolled` is then the PIN and
    /// the surface offers "Sign-in options").
    pub both: bool,
    /// The record is held in the protected persistent store (false: volatile
    /// in-memory store because the protected directory is unavailable).
    pub persisted: bool,
    pub failures: u32,
    /// Ticks until another attempt is accepted (0 = now).
    pub retry_after: u64,
}
impl State {
    pub const BYTES: usize = 16;
    /// Whether `kind` is enrolled.
    pub fn has(&self, kind: Kind) -> bool {
        self.enrolled == Some(kind) || (self.both && self.enrolled.is_some())
    }
    /// Byte 0 is the sign-in kind (as before two kinds existed), byte 2 the
    /// set of enrolled kinds (bit 0 password, bit 1 PIN).
    pub fn encode(&self) -> [u8; Self::BYTES] {
        let mut out = [0; Self::BYTES];
        out[0] = self.enrolled.map_or(0, |kind| kind as u8);
        out[1] = u8::from(self.persisted);
        out[2] = match self.enrolled {
            None => 0,
            Some(_) if self.both => Kind::Password.bit() | Kind::Pin.bit(),
            Some(kind) => kind.bit(),
        };
        out[4..8].copy_from_slice(&self.failures.to_le_bytes());
        out[8..16].copy_from_slice(&self.retry_after.to_le_bytes());
        out
    }
    pub fn decode(bytes: &[u8]) -> Result<Self, Code> {
        let bytes: &[u8; Self::BYTES] = bytes.try_into().map_err(|_| Code::InvalidRequest)?;
        let enrolled = match bytes[0] {
            0 => None,
            other => Some(Kind::from_u8(other).ok_or(Code::InvalidRequest)?),
        };
        let all = Kind::Password.bit() | Kind::Pin.bit();
        let both = match (enrolled, bytes[2]) {
            // A single-kind reply from before two kinds existed carries no set.
            (None, 0) | (Some(_), 0) => false,
            (Some(kind), set) if set == kind.bit() => false,
            (Some(Kind::Pin), set) if set == all => true,
            _ => return Err(Code::InvalidRequest),
        };
        if bytes[1] > 1 || bytes[3] != 0 {
            return Err(Code::InvalidRequest);
        }
        Ok(Self {
            enrolled,
            both,
            persisted: bytes[1] == 1,
            failures: u32::from_le_bytes(bytes[4..8].try_into().unwrap()),
            retry_after: u64::from_le_bytes(bytes[8..16].try_into().unwrap()),
        })
    }
}

/// `CHANGE`, `ADD` and `REMOVE` reply: whether the current secret was
/// accepted (and the update applied), rejected or not checked because of the
/// throttle, and the state afterwards.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Outcome {
    pub verdict: Verdict,
    pub state: State,
}
impl Outcome {
    pub const BYTES: usize = Verdict::BYTES + State::BYTES;
    pub fn encode(&self) -> [u8; Self::BYTES] {
        let mut out = [0; Self::BYTES];
        out[..Verdict::BYTES].copy_from_slice(&self.verdict.encode());
        out[Verdict::BYTES..].copy_from_slice(&self.state.encode());
        out
    }
    pub fn decode(bytes: &[u8]) -> Result<Self, Code> {
        if bytes.len() != Self::BYTES {
            return Err(Code::InvalidRequest);
        }
        let (verdict, state) = bytes.split_at(Verdict::BYTES);
        Ok(Self {
            verdict: Verdict::decode(verdict)?,
            state: State::decode(state)?,
        })
    }
}

/// `VERIFY` reply.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Verdict {
    Accepted,
    /// Wrong secret; `retry_after` is nonzero once throttling has begun.
    Rejected {
        failures: u32,
        retry_after: u64,
    },
    /// Not checked: an attempt arrived before the throttle deadline.
    Throttled {
        failures: u32,
        retry_after: u64,
    },
}
impl Verdict {
    pub const BYTES: usize = 16;
    pub fn encode(&self) -> [u8; Self::BYTES] {
        let (tag, failures, retry) = match *self {
            Self::Accepted => (0u8, 0, 0),
            Self::Rejected {
                failures,
                retry_after,
            } => (1, failures, retry_after),
            Self::Throttled {
                failures,
                retry_after,
            } => (2, failures, retry_after),
        };
        let mut out = [0; Self::BYTES];
        out[0] = tag;
        out[4..8].copy_from_slice(&failures.to_le_bytes());
        out[8..16].copy_from_slice(&retry.to_le_bytes());
        out
    }
    pub fn decode(bytes: &[u8]) -> Result<Self, Code> {
        let bytes: &[u8; Self::BYTES] = bytes.try_into().map_err(|_| Code::InvalidRequest)?;
        if bytes[1..4] != [0, 0, 0] {
            return Err(Code::InvalidRequest);
        }
        let failures = u32::from_le_bytes(bytes[4..8].try_into().unwrap());
        let retry_after = u64::from_le_bytes(bytes[8..16].try_into().unwrap());
        match bytes[0] {
            0 if failures == 0 && retry_after == 0 => Ok(Self::Accepted),
            1 if failures != 0 => Ok(Self::Rejected {
                failures,
                retry_after,
            }),
            2 if retry_after != 0 => Ok(Self::Throttled {
                failures,
                retry_after,
            }),
            _ => Err(Code::InvalidRequest),
        }
    }
}

/// Delay imposed after `failures` consecutive wrong secrets.
pub const fn throttle_delay(failures: u32) -> u64 {
    if failures < FREE_ATTEMPTS {
        return 0;
    }
    let doublings = failures - FREE_ATTEMPTS;
    if doublings >= 4 {
        return MAX_DELAY_TICKS;
    }
    let delay = FIRST_DELAY_TICKS << doublings;
    if delay > MAX_DELAY_TICKS {
        MAX_DELAY_TICKS
    } else {
        delay
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secret_policy_and_framing_round_trip() {
        let mut out = [0; 2 + SECRET_MAX];
        for (kind, secret, ok) in [
            (Kind::Pin, &b"1234"[..], true),
            (Kind::Pin, b"123", false),
            (Kind::Pin, b"12a4", false),
            (Kind::Pin, b"12345678901234567", false),
            (Kind::Password, b"door", true),
            (Kind::Password, "pässwörd".as_bytes(), true),
            (Kind::Password, b"abc", false),
            (Kind::Password, b"ab\ncd", false),
            (Kind::Password, &[0xff, 0xfe, 0x41, 0x42], false),
        ] {
            assert_eq!(validate(kind, secret).is_ok(), ok, "{secret:?}");
            match encode_secret(kind, secret, &mut out) {
                Ok(len) => {
                    assert!(ok);
                    assert_eq!(decode_secret(&out[..len]).unwrap(), (kind, secret));
                }
                Err(_) => assert!(!ok),
            }
        }
        let len = encode_secret(Kind::Pin, b"2468", &mut out).unwrap();
        let mut bad = out;
        bad[1] = 5;
        assert!(decode_secret(&bad[..len]).is_err());
        bad = out;
        bad[0] = 3;
        assert!(decode_secret(&bad[..len]).is_err());
        assert!(decode_secret(&out[..1]).is_err());
    }

    #[test]
    fn state_and_verdict_encodings_are_strict() {
        for state in [
            State {
                enrolled: None,
                both: false,
                persisted: false,
                failures: 0,
                retry_after: 0,
            },
            State {
                enrolled: Some(Kind::Pin),
                both: false,
                persisted: true,
                failures: 7,
                retry_after: 1200,
            },
            State {
                enrolled: Some(Kind::Password),
                both: false,
                persisted: true,
                failures: 0,
                retry_after: 0,
            },
            State {
                enrolled: Some(Kind::Pin),
                both: true,
                persisted: true,
                failures: 2,
                retry_after: 0,
            },
        ] {
            assert_eq!(State::decode(&state.encode()).unwrap(), state);
        }
        let mut bad = State::decode(&[0; 16]).unwrap().encode();
        bad[0] = 9;
        assert!(State::decode(&bad).is_err());
        let both = State {
            enrolled: Some(Kind::Pin),
            both: true,
            persisted: true,
            failures: 0,
            retry_after: 0,
        };
        assert!(both.has(Kind::Pin) && both.has(Kind::Password));
        let encoded = both.encode();
        // Byte 0 still names the kind the surface signs in with.
        assert_eq!((encoded[0], encoded[2]), (Kind::Pin as u8, 3));
        // Both kinds are announced only with the PIN as the sign-in kind, a
        // set always matches byte 0, and nothing is enrolled without a kind.
        for (kind, set) in [(1, 3), (2, 1), (1, 2), (0, 1), (2, 4), (2, 7)] {
            let mut bad = encoded;
            (bad[0], bad[2]) = (kind, set);
            assert!(State::decode(&bad).is_err(), "{kind} {set}");
        }
        // A single-kind reply without the set (the original encoding).
        let mut legacy = encoded;
        legacy[2] = 0;
        let legacy = State::decode(&legacy).unwrap();
        assert!(!legacy.both && legacy.has(Kind::Pin) && !legacy.has(Kind::Password));
        let none = State::decode(&[0; 16]).unwrap();
        assert!(!none.has(Kind::Pin) && !none.has(Kind::Password));
        let outcome = Outcome {
            verdict: Verdict::Rejected {
                failures: 5,
                retry_after: FIRST_DELAY_TICKS,
            },
            state: both,
        };
        assert_eq!(Outcome::decode(&outcome.encode()).unwrap(), outcome);
        assert!(Outcome::decode(&outcome.encode()[..31]).is_err());
        for verdict in [
            Verdict::Accepted,
            Verdict::Rejected {
                failures: 1,
                retry_after: 0,
            },
            Verdict::Throttled {
                failures: 6,
                retry_after: 3000,
            },
        ] {
            assert_eq!(Verdict::decode(&verdict.encode()).unwrap(), verdict);
        }
        let mut forged = Verdict::Accepted.encode();
        forged[4] = 1;
        assert!(Verdict::decode(&forged).is_err());
        assert!(Verdict::decode(&[1; 16]).is_err());
    }

    #[test]
    fn update_framing_checks_both_secrets_against_their_kinds() {
        assert_eq!(UPDATE_MAX, 131);
        for update in [Update::Change, Update::Add, Update::Remove] {
            assert_eq!(Update::from_opcode(update.opcode()), Some(update));
        }
        assert_eq!(Update::from_opcode(VERIFY), None);
        assert_eq!(Update::Change.proof(Kind::Pin), Kind::Pin);
        assert_eq!(Update::Add.proof(Kind::Pin), Kind::Password);
        assert_eq!(Update::Remove.proof(Kind::Password), Kind::Pin);
        let mut out = [0; UPDATE_MAX];
        let long = [b'x'; SECRET_MAX];
        for (update, kind, current, new, ok) in [
            (
                Update::Change,
                Kind::Pin,
                &b"2468"[..],
                &b"135790"[..],
                true,
            ),
            (Update::Change, Kind::Password, &long[..], &long[..], true),
            // CHANGE proves with the same kind.
            (Update::Change, Kind::Pin, b"door", b"1357", false),
            (Update::Change, Kind::Pin, b"2468", b"12", false),
            // ADD a PIN with the password, a password with the PIN.
            (Update::Add, Kind::Pin, b"door-os!", b"1357", true),
            (Update::Add, Kind::Password, b"2468", b"door-os!", true),
            (Update::Add, Kind::Pin, b"abc", b"1357", false),
            (Update::Add, Kind::Password, b"2468", b"abc", false),
            // REMOVE carries only the proof of the other kind.
            (Update::Remove, Kind::Pin, b"door-os!", b"", true),
            (Update::Remove, Kind::Password, b"2468", b"", true),
            (Update::Remove, Kind::Pin, b"door-os!", b"1357", false),
            (Update::Remove, Kind::Pin, b"24", b"", false),
        ] {
            match encode_update(update, kind, current, new, &mut out) {
                Ok(len) => {
                    assert!(ok, "{update:?} {kind:?}");
                    assert_eq!(len, 3 + current.len() + new.len());
                    assert_eq!(
                        decode_update(update, &out[..len]).unwrap(),
                        (kind, current, new)
                    );
                }
                Err(code) => {
                    assert!(!ok, "{update:?} {kind:?}");
                    assert_eq!(code, Code::InvalidRequest);
                }
            }
        }
        let len = encode_update(Update::Change, Kind::Pin, b"2468", b"1357", &mut out).unwrap();
        let body = out;
        for (index, value) in [(0, 0), (0, 3), (1, 5), (2, 3), (2, 5)] {
            let mut bad = body;
            bad[index] = value;
            assert!(decode_update(Update::Change, &bad[..len]).is_err());
        }
        assert!(decode_update(Update::Change, &body[..len - 1]).is_err());
        assert!(decode_update(Update::Change, &body[..2]).is_err());
        // A REMOVE never carries a new secret; a password CHANGE is no ADD.
        assert!(decode_update(Update::Remove, &body[..len]).is_err());
        let len =
            encode_update(Update::Change, Kind::Password, b"door", b"os!!", &mut out).unwrap();
        assert!(decode_update(Update::Add, &out[..len]).is_err());
        assert!(encode_update(Update::Change, Kind::Pin, b"2468", b"1357", &mut out[..8]).is_err());
    }

    #[test]
    fn throttle_grows_and_caps() {
        assert_eq!(throttle_delay(0), 0);
        assert_eq!(throttle_delay(FREE_ATTEMPTS - 1), 0);
        assert_eq!(throttle_delay(FREE_ATTEMPTS), FIRST_DELAY_TICKS);
        assert_eq!(throttle_delay(FREE_ATTEMPTS + 1), 2 * FIRST_DELAY_TICKS);
        assert_eq!(throttle_delay(FREE_ATTEMPTS + 3), 8 * FIRST_DELAY_TICKS);
        assert_eq!(throttle_delay(FREE_ATTEMPTS + 4), MAX_DELAY_TICKS);
        assert_eq!(throttle_delay(u32::MAX), MAX_DELAY_TICKS);
    }
}

/// The sign-in surface picks whose credentials STATE/ENROLL/VERIFY act on:
/// `[uid u32]` (an account `ACCOUNTS` lists). Settings always acts on its
/// own user's.
pub const SELECT: u32 = 0x4016;
/// The local accounts: reply `Accounts::encode`. Surface, Init, Settings.
pub const ACCOUNTS: u32 = 0x4017;
/// Settings of an administrator: a new local account with a password,
/// `[name_len u8][name][password_len u8][password]`; reply `[uid u32]`.
/// New accounts are standard users, as Windows makes them.
pub const ACCOUNT_CREATE: u32 = 0x4018;
/// Settings of an administrator: `[uid u32][administrator u8]`. The last
/// administrator cannot become a standard user.
pub const ACCOUNT_TYPE: u32 = 0x4019;
/// Settings of an administrator: `[uid u32]`, another account. Its
/// credentials and record go; its files stay in its profile folder.
pub const ACCOUNT_DELETE: u32 = 0x401a;

/// Account names: Windows' 20-character local user name limit.
pub const NAME_MAX: usize = 20;
/// The accounts the kernel provisions logon permits for.
pub const ACCOUNT_UIDS: [u32; 2] = [1001, 1002];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Account {
    pub uid: u32,
    pub administrator: bool,
    name: [u8; NAME_MAX * 4],
    name_len: u8,
}
impl Account {
    pub fn new(uid: u32, name: &str, administrator: bool) -> Result<Self, Code> {
        if !valid_name(name) || !ACCOUNT_UIDS.contains(&uid) {
            return Err(Code::InvalidRequest);
        }
        let mut bytes = [0; NAME_MAX * 4];
        bytes[..name.len()].copy_from_slice(name.as_bytes());
        Ok(Self {
            uid,
            administrator,
            name: bytes,
            name_len: name.len() as u8,
        })
    }
    pub fn name(&self) -> &str {
        core::str::from_utf8(&self.name[..usize::from(self.name_len)]).unwrap_or("")
    }
}
/// A local user name Windows accepts: 1..=20 characters, not only dots or
/// spaces, none of `" / \ [ ] : ; | = , + * ? < > @`, no control characters.
pub fn valid_name(name: &str) -> bool {
    let count = name.chars().count();
    (1..=NAME_MAX).contains(&count)
        && name.len() <= NAME_MAX * 4
        && !name.chars().all(|c| c == '.' || c == ' ')
        && !name
            .chars()
            .any(|c| c.is_control() || "\"/\\[]:;|=,+*?<>@".contains(c))
}
/// `ACCOUNTS` reply: `[count u8]` then per account `[uid u32][flags u8 (bit0
/// administrator)][name_len u8][name]`.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Accounts {
    pub list: [Option<Account>; 2],
}
impl Accounts {
    pub const MAX_BYTES: usize = 1 + 2 * (6 + NAME_MAX * 4);
    pub fn iter(&self) -> impl Iterator<Item = &Account> {
        self.list.iter().flatten()
    }
    pub fn get(&self, uid: u32) -> Option<&Account> {
        self.iter().find(|a| a.uid == uid)
    }
    pub fn administrators(&self) -> usize {
        self.iter().filter(|a| a.administrator).count()
    }
    pub fn encode(&self, out: &mut [u8; Self::MAX_BYTES]) -> usize {
        let mut at = 1;
        let mut count = 0;
        for a in self.iter() {
            out[at..at + 4].copy_from_slice(&a.uid.to_le_bytes());
            out[at + 4] = u8::from(a.administrator);
            out[at + 5] = a.name_len;
            let n = usize::from(a.name_len);
            out[at + 6..at + 6 + n].copy_from_slice(&a.name[..n]);
            at += 6 + n;
            count += 1;
        }
        out[0] = count;
        at
    }
    pub fn decode(bytes: &[u8]) -> Result<Self, Code> {
        let (&count, mut rest) = bytes.split_first().ok_or(Code::InvalidRequest)?;
        if count > 2 {
            return Err(Code::InvalidRequest);
        }
        let mut list = [None; 2];
        for slot in list.iter_mut().take(usize::from(count)) {
            if rest.len() < 6 || rest[4] > 1 {
                return Err(Code::InvalidRequest);
            }
            let uid = u32::from_le_bytes(rest[..4].try_into().unwrap());
            let n = usize::from(rest[5]);
            let name = rest
                .get(6..6 + n)
                .and_then(|b| core::str::from_utf8(b).ok())
                .ok_or(Code::InvalidRequest)?;
            *slot = Some(Account::new(uid, name, rest[4] == 1)?);
            rest = &rest[6 + n..];
        }
        if !rest.is_empty()
            || (list[0].is_some() && list[1].is_some_and(|b| Some(b.uid) == list[0].map(|a| a.uid)))
        {
            return Err(Code::InvalidRequest);
        }
        Ok(Self { list })
    }
}
