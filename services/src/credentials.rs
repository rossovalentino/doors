//! Local credential store of the session service (P14-LOGON, ADR 0020).
//!
//! The local user has a password, a PIN, or both. Each is one record:
//! Argon2id (RustCrypto `argon2`, pinned) over the secret with a 16-byte salt
//! drawn from the kernel entropy source, the cost parameters and the 32-byte
//! tag. Nothing here ever returns a secret, a tag or a salt to a caller: the
//! service answers accept/reject/throttle. The Argon2 working memory is
//! supplied by the caller (the session binary owns the heap) and is wiped
//! after every use; candidate tags and replaced records are wiped too.
//! Throttling lives in the service and is shared by every operation that
//! checks a secret (sign-in, unlock, change, add, remove), so neither a
//! restarted sign-in surface nor switching operations or kinds can reset it.
//!
//! Store format v2 (`/state/credentials/local.v1`): one or two `DOORCRD1`
//! records, at most one per kind, in ascending kind order (password, then
//! PIN). A single-kind v2 store is byte-identical to the original v1 record,
//! so v1 stores load unchanged and migrate without a rewrite. Every write
//! replaces the whole store atomically.
use argon2::{Algorithm, Argon2, Block, Params as Argon2Params, Version};
use door_service_protocol::{
    Code,
    credential::{self, Kind, Outcome, State, Update, Verdict},
};

/// Production cost: 4 MiB, three passes, one lane (ADR 0020 section 3).
pub const PARAMS: Params = Params {
    m_kib: 4096,
    t_cost: 3,
    lanes: 1,
};
pub const SALT_BYTES: usize = 16;
pub const TAG_BYTES: usize = 32;
pub const RECORD_BYTES: usize = 96;
/// The largest store: a password record and a PIN record.
pub const STORE_BYTES: usize = 2 * RECORD_BYTES;
const MAGIC: [u8; 8] = *b"DOORCRD1";
/// The single local account of the current session model.
pub const LOCAL_UID: u32 = 1001;
/// A request that has already spent this many ticks (1.5 s) does not start a
/// second Argon2 derivation: CHANGE and ADD verify the current secret, then
/// derive the new one, and both must finish with the write well inside the
/// client's 5 s reply deadline (`door_service_client` rpc). The caller is
/// told to retry (`WouldBlock`) and nothing changes.
pub const SECOND_DERIVATION_BUDGET: u64 = 150;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Params {
    pub m_kib: u32,
    pub t_cost: u32,
    pub lanes: u32,
}
impl Params {
    fn argon2(self) -> Result<Argon2Params, Code> {
        Argon2Params::new(self.m_kib, self.t_cost, self.lanes, Some(TAG_BYTES))
            .map_err(|_| Code::InvalidRequest)
    }
    /// Argon2 blocks needed for these parameters.
    pub fn blocks(self) -> Result<usize, Code> {
        Ok(self.argon2()?.block_count())
    }
    fn plausible(self) -> bool {
        (8..=65_536).contains(&self.m_kib) && (1..=16).contains(&self.t_cost) && self.lanes == 1
    }
}

/// Clears secret-derived bytes in a way the optimizer keeps.
fn wipe(bytes: &mut [u8]) {
    bytes.fill(0);
    core::hint::black_box(bytes);
}

/// One credential. Not `Copy`: copies would escape the wipe on drop.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Record {
    pub uid: u32,
    pub kind: Kind,
    pub params: Params,
    pub salt: [u8; SALT_BYTES],
    pub tag: [u8; TAG_BYTES],
}
impl Drop for Record {
    fn drop(&mut self) {
        wipe(&mut self.salt);
        wipe(&mut self.tag);
    }
}
impl Record {
    /// `MAGIC, uid, kind, pad, m, t, lanes, salt, tag, pad, fnv64(previous)`.
    /// The checksum detects torn or damaged bytes; confidentiality and
    /// integrity against callers come from the protected directory ACL.
    pub fn encode(&self) -> [u8; RECORD_BYTES] {
        let mut out = [0; RECORD_BYTES];
        out[..8].copy_from_slice(&MAGIC);
        out[8..12].copy_from_slice(&self.uid.to_le_bytes());
        out[12] = self.kind as u8;
        out[16..20].copy_from_slice(&self.params.m_kib.to_le_bytes());
        out[20..24].copy_from_slice(&self.params.t_cost.to_le_bytes());
        out[24..28].copy_from_slice(&self.params.lanes.to_le_bytes());
        out[28..44].copy_from_slice(&self.salt);
        out[44..76].copy_from_slice(&self.tag);
        let sum = fnv64(&out[..88]);
        out[88..96].copy_from_slice(&sum.to_le_bytes());
        out
    }
    pub fn decode(bytes: &[u8]) -> Result<Self, Code> {
        let bytes: &[u8; RECORD_BYTES] = bytes.try_into().map_err(|_| Code::Corrupt)?;
        let word = |at: usize| u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap());
        if bytes[..8] != MAGIC
            || bytes[13..16] != [0; 3]
            || bytes[76..88] != [0; 12]
            || u64::from_le_bytes(bytes[88..96].try_into().unwrap()) != fnv64(&bytes[..88])
        {
            return Err(Code::Corrupt);
        }
        let params = Params {
            m_kib: word(16),
            t_cost: word(20),
            lanes: word(24),
        };
        let kind = Kind::from_u8(bytes[12]).ok_or(Code::Corrupt)?;
        if !params.plausible() || word(8) == 0 {
            return Err(Code::Corrupt);
        }
        Ok(Self {
            uid: word(8),
            kind,
            params,
            salt: bytes[28..44].try_into().unwrap(),
            tag: bytes[44..76].try_into().unwrap(),
        })
    }
}

/// The records of the local account, at most one per kind.
#[derive(Default, Debug, Eq, PartialEq)]
pub struct Records {
    pub password: Option<Record>,
    pub pin: Option<Record>,
}
impl Records {
    pub fn get(&self, kind: Kind) -> Option<&Record> {
        match kind {
            Kind::Password => self.password.as_ref(),
            Kind::Pin => self.pin.as_ref(),
        }
    }
    fn slot(&mut self, kind: Kind) -> &mut Option<Record> {
        match kind {
            Kind::Password => &mut self.password,
            Kind::Pin => &mut self.pin,
        }
    }
    pub fn is_empty(&self) -> bool {
        self.password.is_none() && self.pin.is_none()
    }
    fn only(&self) -> Option<&Record> {
        match (&self.password, &self.pin) {
            (Some(record), None) | (None, Some(record)) => Some(record),
            _ => None,
        }
    }
    /// Store format v2 (module documentation). Returns the encoded length;
    /// an empty set encodes to nothing.
    pub fn encode(&self, out: &mut [u8; STORE_BYTES]) -> usize {
        let mut len = 0;
        for record in [&self.password, &self.pin].into_iter().flatten() {
            out[len..len + RECORD_BYTES].copy_from_slice(&record.encode());
            len += RECORD_BYTES;
        }
        len
    }
    /// Reads a v1 record or a v2 store. An empty store (a first enrollment
    /// interrupted between creating and writing the file) has no records.
    pub fn decode(bytes: &[u8]) -> Result<Self, Code> {
        Self::decode_for(bytes, LOCAL_UID)
    }
    /// As `decode`, for the account `uid`.
    pub fn decode_for(bytes: &[u8], uid: u32) -> Result<Self, Code> {
        if !bytes.len().is_multiple_of(RECORD_BYTES) || bytes.len() > STORE_BYTES {
            return Err(Code::Corrupt);
        }
        let mut records = Self::default();
        let mut previous = 0u8;
        for chunk in bytes.chunks_exact(RECORD_BYTES) {
            let record = Record::decode(chunk)?;
            // One account, each kind once, in ascending kind order.
            if record.uid != uid || record.kind as u8 <= previous {
                return Err(Code::Corrupt);
            }
            previous = record.kind as u8;
            let kind = record.kind;
            *records.slot(kind) = Some(record);
        }
        Ok(records)
    }
}

fn fnv64(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, &byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

/// Argon2id tag of `secret`. `memory` is wiped before returning.
pub fn derive(
    params: Params,
    salt: &[u8; SALT_BYTES],
    secret: &[u8],
    memory: &mut [Block],
) -> Result<[u8; TAG_BYTES], Code> {
    let context = Argon2::new(Algorithm::Argon2id, Version::V0x13, params.argon2()?);
    let mut tag = [0; TAG_BYTES];
    let result = context.hash_password_into_with_memory(secret, salt, &mut tag, &mut *memory);
    for block in memory.iter_mut() {
        *block = Block::new();
    }
    result.map_err(|_| Code::Resources)?;
    Ok(tag)
}

/// Constant-time over the full tag length.
fn tags_equal(a: &[u8; TAG_BYTES], b: &[u8; TAG_BYTES]) -> bool {
    let mut difference = 0u8;
    for (x, y) in a.iter().zip(b) {
        difference |= x ^ y;
    }
    core::hint::black_box(difference) == 0
}

/// Whether `secret` is the secret of `record`. Input outside the record's
/// policy (a password at a PIN record) is simply not it.
fn matches(record: &Record, secret: &[u8], memory: &mut [Block]) -> Result<bool, Code> {
    if credential::validate(record.kind, secret).is_err() {
        return Ok(false);
    }
    let mut candidate = derive(record.params, &record.salt, secret, memory)?;
    let equal = tags_equal(&candidate, &record.tag);
    wipe(&mut candidate);
    Ok(equal)
}

/// Where the encoded store lives. `Unavailable` selects a volatile store.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PersistError {
    /// The protected credential directory is not provisioned (or denied).
    Unavailable,
    /// The store exists but could not be read or written.
    Failed,
}
pub trait Persist {
    /// The stored bytes, copied into `out`: `Ok(None)` when no store exists
    /// yet, `Ok(Some(len))` otherwise (`len` may be zero). A store longer
    /// than `STORE_BYTES` is `Failed`.
    fn load(&mut self, out: &mut [u8; STORE_BYTES]) -> Result<Option<usize>, PersistError>;
    /// Replaces the whole store with `bytes` (never empty) atomically: after
    /// a crash the store holds either the previous bytes or `bytes`. An
    /// error leaves the outcome uncertain; the store reloads before its next
    /// use.
    fn save(&mut self, bytes: &[u8]) -> Result<(), PersistError>;
}

/// Store state machine. `entropy` fills a salt from the kernel source.
pub struct Store<P: Persist> {
    persist: P,
    params: Params,
    /// The account whose credentials these are.
    uid: u32,
    records: Records,
    loaded: bool,
    persisted: bool,
    /// A damaged persistent store fails closed: no enrollment over it.
    damaged: bool,
    failures: u32,
    retry_at: u64,
}
impl<P: Persist> Store<P> {
    pub fn new(persist: P, params: Params) -> Self {
        Self::for_account(persist, params, LOCAL_UID)
    }
    pub fn for_account(persist: P, params: Params, uid: u32) -> Self {
        Self {
            persist,
            params,
            uid,
            records: Records::default(),
            loaded: false,
            persisted: false,
            damaged: false,
            failures: 0,
            retry_at: 0,
        }
    }
    /// Loads once (and again after an uncertain write); `Unavailable` keeps a
    /// volatile store and is retried on the next request until a protected
    /// directory appears.
    fn load(&mut self) {
        if self.loaded {
            return;
        }
        let mut bytes = [0; STORE_BYTES];
        match self.persist.load(&mut bytes) {
            Ok(found) => {
                let stored = bytes.get(..found.unwrap_or(0)).ok_or(Code::Corrupt);
                match stored.and_then(|b| Records::decode_for(b, self.uid)) {
                    Ok(records) => {
                        self.records = records;
                        self.damaged = false;
                    }
                    Err(_) => {
                        self.records = Records::default();
                        self.damaged = true;
                    }
                }
                self.persisted = true;
                self.loaded = true;
            }
            Err(PersistError::Unavailable) => self.persisted = false,
            Err(PersistError::Failed) => {
                self.records = Records::default();
                self.damaged = true;
                self.loaded = true;
            }
        }
        wipe(&mut bytes);
    }
    fn ready(&mut self) -> Result<(), Code> {
        self.load();
        if self.damaged {
            Err(Code::Corrupt)
        } else {
            Ok(())
        }
    }
    fn snapshot(&self, now: u64) -> State {
        let both = self.records.password.is_some() && self.records.pin.is_some();
        State {
            enrolled: [Kind::Pin, Kind::Password]
                .into_iter()
                .find(|&kind| self.records.get(kind).is_some()),
            both,
            persisted: self.persisted,
            failures: self.failures,
            retry_after: self.retry_at.saturating_sub(now),
        }
    }
    pub fn state(&mut self, now: u64) -> Result<State, Code> {
        self.ready()?;
        Ok(self.snapshot(now))
    }
    fn fresh(
        &self,
        kind: Kind,
        secret: &[u8],
        entropy: &mut dyn FnMut(&mut [u8]) -> Result<(), Code>,
        memory: &mut [Block],
    ) -> Result<Record, Code> {
        credential::validate(kind, secret)?;
        let mut salt = [0; SALT_BYTES];
        entropy(&mut salt)?;
        if salt.iter().all(|&b| b == salt[0]) {
            // A stuck source must never produce a predictable salt.
            return Err(Code::Resources);
        }
        let tag = derive(self.params, &salt, secret, memory)?;
        Ok(Record {
            uid: self.uid,
            kind,
            params: self.params,
            salt,
            tag,
        })
    }
    /// Writes `records` as the whole store (when persistent), then makes them
    /// current. On a failed write nothing changes in memory and the store
    /// reloads before its next use, because the write may have landed.
    fn commit(&mut self, records: Records) -> Result<(), Code> {
        if self.persisted {
            let mut bytes = [0; STORE_BYTES];
            let len = records.encode(&mut bytes);
            let saved = self.persist.save(&bytes[..len]);
            wipe(&mut bytes);
            if saved.is_err() {
                self.loaded = false;
                return Err(Code::Io);
            }
        }
        // The previous records drop (and wipe) here.
        self.records = records;
        Ok(())
    }
    fn failed(&mut self, now: u64) -> Verdict {
        self.failures = self.failures.saturating_add(1);
        let delay = credential::throttle_delay(self.failures);
        self.retry_at = now.saturating_add(delay);
        Verdict::Rejected {
            failures: self.failures,
            retry_after: delay,
        }
    }
    fn throttled(&self, now: u64) -> Option<Verdict> {
        (now < self.retry_at).then(|| Verdict::Throttled {
            failures: self.failures,
            retry_after: self.retry_at - now,
        })
    }
    /// The account was deleted: its credentials go (an empty store, as an
    /// interrupted first enrollment leaves), so a new account in its place
    /// enrolls afresh.
    pub fn clear(&mut self) -> Result<(), Code> {
        self.load();
        self.damaged = false;
        self.failures = 0;
        self.retry_at = 0;
        self.commit(Records::default())
    }
    /// First-run enrollment of the account's first credential.
    pub fn enroll(
        &mut self,
        kind: Kind,
        secret: &[u8],
        entropy: &mut dyn FnMut(&mut [u8]) -> Result<(), Code>,
        memory: &mut [Block],
    ) -> Result<State, Code> {
        self.ready()?;
        if !self.records.is_empty() {
            return Err(Code::Exists);
        }
        let record = self.fresh(kind, secret, entropy, memory)?;
        let mut records = Records::default();
        *records.slot(kind) = Some(record);
        self.commit(records)?;
        // A volatile enrollment stays volatile for this service instance.
        self.loaded = true;
        self.failures = 0;
        self.retry_at = 0;
        Ok(self.snapshot(0))
    }
    /// Sign-in and unlock. With one kind enrolled the secret is checked
    /// against it whatever kind the caller framed it as (the reply never
    /// reveals which policy the record uses); with both, against `kind`.
    pub fn verify(
        &mut self,
        kind: Kind,
        secret: &[u8],
        now: u64,
        memory: &mut [Block],
    ) -> Result<Verdict, Code> {
        self.ready()?;
        if self.records.is_empty() {
            return Err(Code::NotFound);
        }
        if let Some(verdict) = self.throttled(now) {
            return Ok(verdict);
        }
        let record = self
            .records
            .only()
            .or(self.records.get(kind))
            .ok_or(Code::NotFound)?;
        if matches(record, secret, memory)? {
            self.failures = 0;
            self.retry_at = 0;
            return Ok(Verdict::Accepted);
        }
        Ok(self.failed(now))
    }
    /// `CHANGE`, `ADD` or `REMOVE` the credential of `kind`, proved with
    /// `current`, the secret of `update.proof(kind)`. The proof is checked
    /// first, under the shared throttle, and only an accepted proof changes
    /// anything. `clock` reads the time again before a second derivation.
    #[allow(clippy::too_many_arguments)]
    pub fn update(
        &mut self,
        update: Update,
        kind: Kind,
        current: &[u8],
        new: &[u8],
        now: u64,
        clock: &mut dyn FnMut() -> u64,
        entropy: &mut dyn FnMut(&mut [u8]) -> Result<(), Code>,
        memory: &mut [Block],
    ) -> Result<Outcome, Code> {
        self.ready()?;
        if update.has_new() {
            credential::validate(kind, new)?;
        } else if !new.is_empty() {
            return Err(Code::InvalidRequest);
        }
        let proof = update.proof(kind);
        // Enrollment is no secret to this caller (STATE reports it). REMOVE
        // needs the other kind, so the last credential can never go.
        match update {
            Update::Add if self.records.get(kind).is_some() => return Err(Code::Exists),
            Update::Remove if self.records.get(kind).is_none() => return Err(Code::NotFound),
            _ => (),
        }
        if self.records.get(proof).is_none() {
            return Err(Code::NotFound);
        }
        let outcome = |store: &Self, verdict| Outcome {
            verdict,
            state: store.snapshot(now),
        };
        if let Some(verdict) = self.throttled(now) {
            return Ok(outcome(self, verdict));
        }
        let record = self.records.get(proof).ok_or(Code::NotFound)?;
        if !matches(record, current, memory)? {
            let verdict = self.failed(now);
            return Ok(outcome(self, verdict));
        }
        self.failures = 0;
        self.retry_at = 0;
        let mut records = Records {
            password: self.records.password.clone(),
            pin: self.records.pin.clone(),
        };
        if update.has_new() {
            if clock().saturating_sub(now) > SECOND_DERIVATION_BUDGET {
                return Err(Code::WouldBlock);
            }
            *records.slot(kind) = Some(self.fresh(kind, new, entropy, memory)?);
        } else {
            *records.slot(kind) = None;
        }
        self.commit(records)?;
        Ok(outcome(self, Verdict::Accepted))
    }
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use std::vec;
    use std::vec::Vec;

    const TEST: Params = Params {
        m_kib: 8,
        t_cost: 1,
        lanes: 1,
    };
    #[derive(Default)]
    struct Memory {
        bytes: Option<Vec<u8>>,
        unavailable: bool,
        fail_save: bool,
        saves: usize,
    }
    impl Persist for &mut Memory {
        fn load(&mut self, out: &mut [u8; STORE_BYTES]) -> Result<Option<usize>, PersistError> {
            if self.unavailable {
                return Err(PersistError::Unavailable);
            }
            match &self.bytes {
                None => Ok(None),
                Some(bytes) if bytes.len() > STORE_BYTES => Err(PersistError::Failed),
                Some(bytes) => {
                    out[..bytes.len()].copy_from_slice(bytes);
                    Ok(Some(bytes.len()))
                }
            }
        }
        fn save(&mut self, bytes: &[u8]) -> Result<(), PersistError> {
            assert!(!bytes.is_empty() && bytes.len() <= STORE_BYTES);
            if self.fail_save {
                return Err(PersistError::Failed);
            }
            self.bytes = Some(bytes.to_vec());
            self.saves += 1;
            Ok(())
        }
    }
    fn counter() -> impl FnMut(&mut [u8]) -> Result<(), Code> {
        let mut next = 7u8;
        move |out: &mut [u8]| {
            for byte in out {
                next = next.wrapping_mul(31).wrapping_add(11);
                *byte = next;
            }
            Ok(())
        }
    }
    fn blocks(params: Params) -> Vec<Block> {
        vec![Block::new(); params.blocks().unwrap()]
    }
    fn update<P: Persist>(
        store: &mut Store<P>,
        update: Update,
        kind: Kind,
        current: &[u8],
        new: &[u8],
        now: u64,
        memory: &mut [Block],
    ) -> Result<Outcome, Code> {
        let mut clock = move || now;
        store.update(
            update,
            kind,
            current,
            new,
            now,
            &mut clock,
            &mut counter(),
            memory,
        )
    }
    fn verdict(outcome: Result<Outcome, Code>) -> Result<Verdict, Code> {
        outcome.map(|o| o.verdict)
    }

    #[test]
    fn derive_is_deterministic_argon2id_v13_and_wipes_memory() {
        // The pinned crate's own known-answer tests cover RFC 9106. Here the
        // store's output must equal a direct Argon2id v0x13 computation, be
        // salt-dependent and leave no derived material in the working memory.
        let params = Params {
            m_kib: 32,
            t_cost: 3,
            lanes: 1,
        };
        let mut memory = blocks(params);
        let tag = derive(params, &[2; SALT_BYTES], &[1; 32], &mut memory).unwrap();
        let again = derive(params, &[2; SALT_BYTES], &[1; 32], &mut memory).unwrap();
        assert_eq!(tag, again);
        assert_ne!(
            tag,
            derive(params, &[3; SALT_BYTES], &[1; 32], &mut memory).unwrap()
        );
        assert!(
            memory.iter().all(|b| b.as_ref().iter().all(|&w| w == 0)),
            "memory wiped"
        );
        let reference = Argon2::new(
            Algorithm::Argon2id,
            Version::V0x13,
            params.argon2().unwrap(),
        );
        let mut expected = [0; TAG_BYTES];
        reference
            .hash_password_into_with_memory(&[1; 32], &[2; SALT_BYTES], &mut expected, &mut memory)
            .unwrap();
        assert_eq!(tag, expected);
    }

    #[test]
    fn enroll_verify_throttle_and_persist() {
        let mut disk = Memory::default();
        let mut memory = blocks(TEST);
        let mut entropy = counter();
        {
            let mut store = Store::new(&mut disk, TEST);
            let state = store.state(0).unwrap();
            assert_eq!((state.enrolled, state.persisted), (None, true));
            assert_eq!(
                store.verify(Kind::Pin, b"1234", 0, &mut memory),
                Err(Code::NotFound)
            );
            assert!(
                store
                    .enroll(Kind::Pin, b"12", &mut entropy, &mut memory)
                    .is_err()
            );
            let state = store
                .enroll(Kind::Pin, b"2468", &mut entropy, &mut memory)
                .unwrap();
            assert_eq!((state.enrolled, state.both), (Some(Kind::Pin), false));
            assert_eq!(
                store.enroll(Kind::Pin, b"1357", &mut entropy, &mut memory),
                Err(Code::Exists)
            );
            assert_eq!(
                store.enroll(Kind::Password, b"door-os!", &mut entropy, &mut memory),
                Err(Code::Exists)
            );
            assert_eq!(
                store.verify(Kind::Pin, b"2468", 10, &mut memory),
                Ok(Verdict::Accepted)
            );
            for failures in 1..credential::FREE_ATTEMPTS {
                assert_eq!(
                    store.verify(Kind::Pin, b"1111", 10, &mut memory),
                    Ok(Verdict::Rejected {
                        failures,
                        retry_after: 0
                    })
                );
            }
            // A password-shaped guess against a PIN record is just wrong.
            assert_eq!(
                store.verify(Kind::Password, b"letmein", 10, &mut memory),
                Ok(Verdict::Rejected {
                    failures: credential::FREE_ATTEMPTS,
                    retry_after: credential::FIRST_DELAY_TICKS
                })
            );
            // Even the right PIN is not checked before the deadline.
            assert!(matches!(
                store.verify(Kind::Pin, b"2468", 11, &mut memory),
                Ok(Verdict::Throttled { .. })
            ));
            let later = 10 + credential::FIRST_DELAY_TICKS;
            assert_eq!(
                store.verify(Kind::Pin, b"2468", later, &mut memory),
                Ok(Verdict::Accepted)
            );
            assert_eq!(store.state(later).unwrap().failures, 0);
        }
        assert_eq!(disk.saves, 1);
        let saved = disk.bytes.clone().unwrap();
        assert_eq!(saved.len(), RECORD_BYTES);
        // The record holds no plaintext secret.
        assert!(!saved.windows(4).any(|w| w == b"2468"));
        // A new service instance reloads it.
        let mut store = Store::new(&mut disk, TEST);
        assert_eq!(store.state(0).unwrap().enrolled, Some(Kind::Pin));
        assert_eq!(
            store.verify(Kind::Pin, b"2468", 0, &mut memory),
            Ok(Verdict::Accepted)
        );
    }

    #[test]
    fn v1_records_load_unchanged_and_migrate_to_two_kinds() {
        let mut memory = blocks(TEST);
        // A v1 store exactly as the original store wrote it: one record.
        let v1 = Store::new(&mut Memory::default(), TEST)
            .fresh(Kind::Pin, b"2468", &mut counter(), &mut memory)
            .unwrap()
            .encode();
        let mut disk = Memory {
            bytes: Some(v1.to_vec()),
            ..Memory::default()
        };
        {
            let mut store = Store::new(&mut disk, TEST);
            let state = store.state(0).unwrap();
            assert_eq!(
                (state.enrolled, state.both, state.persisted),
                (Some(Kind::Pin), false, true)
            );
            assert_eq!(
                store.verify(Kind::Pin, b"2468", 0, &mut memory),
                Ok(Verdict::Accepted)
            );
            // Add a password with the PIN: the store becomes v2.
            let outcome = update(
                &mut store,
                Update::Add,
                Kind::Password,
                b"2468",
                b"door-os!",
                1,
                &mut memory,
            )
            .unwrap();
            assert_eq!(outcome.verdict, Verdict::Accepted);
            assert_eq!(
                (outcome.state.enrolled, outcome.state.both),
                (Some(Kind::Pin), true)
            );
        }
        let v2 = disk.bytes.clone().unwrap();
        assert_eq!(v2.len(), STORE_BYTES);
        // Password first, then the untouched v1 PIN record.
        assert_eq!(v2[12], Kind::Password as u8);
        assert_eq!(&v2[RECORD_BYTES..], &v1[..]);
        assert!(!v2.windows(8).any(|w| w == b"door-os!"));
        let records = Records::decode(&v2).unwrap();
        assert!(records.password.is_some() && records.pin.is_some());
        let mut again = [0; STORE_BYTES];
        assert_eq!(records.encode(&mut again), STORE_BYTES);
        assert_eq!(&again[..], &v2[..]);
        // A new instance signs in with either kind.
        let mut store = Store::new(&mut disk, TEST);
        assert!(store.state(0).unwrap().both);
        for (kind, secret) in [(Kind::Pin, &b"2468"[..]), (Kind::Password, b"door-os!")] {
            assert_eq!(
                store.verify(kind, secret, 0, &mut memory),
                Ok(Verdict::Accepted)
            );
        }
        // With two kinds the framed kind selects the record.
        assert!(matches!(
            store.verify(Kind::Password, b"2468", 0, &mut memory),
            Ok(Verdict::Rejected { failures: 1, .. })
        ));
        assert!(matches!(
            store.verify(Kind::Pin, b"door-os!", 0, &mut memory),
            Ok(Verdict::Rejected { failures: 2, .. })
        ));
    }

    #[test]
    fn store_format_is_strict() {
        let mut memory = blocks(TEST);
        let mut scratch = Memory::default();
        let store = Store::new(&mut scratch, TEST);
        let pin = store
            .fresh(Kind::Pin, b"2468", &mut counter(), &mut memory)
            .unwrap()
            .encode();
        let password = store
            .fresh(Kind::Password, b"door-os!", &mut counter(), &mut memory)
            .unwrap()
            .encode();
        let join = |a: &[u8], b: &[u8]| [a, b].concat();
        assert!(Records::decode(&[]).unwrap().is_empty());
        assert!(Records::decode(&join(&password, &pin)).is_ok());
        for bad in [
            join(&pin, &password),
            join(&pin, &pin),
            join(&password, &password),
            pin[..95].to_vec(),
            join(&join(&password, &pin), &pin),
            join(&password, &pin[..50]),
        ] {
            assert_eq!(Records::decode(&bad), Err(Code::Corrupt), "{}", bad.len());
        }
        // A record of another account is refused even when well formed.
        let mut foreign = Records::decode(&pin).unwrap().pin.unwrap();
        foreign.uid = 1002;
        assert_eq!(Records::decode(&foreign.encode()), Err(Code::Corrupt));
    }

    #[test]
    fn change_add_remove_verify_first_and_never_leave_no_credential() {
        let mut memory = blocks(TEST);
        let mut entropy = counter();
        let mut disk = Memory::default();
        let mut store = Store::new(&mut disk, TEST);
        // Nothing to change before enrollment.
        assert_eq!(
            update(
                &mut store,
                Update::Change,
                Kind::Pin,
                b"2468",
                b"1357",
                0,
                &mut memory
            ),
            Err(Code::NotFound)
        );
        store
            .enroll(Kind::Password, b"door-os!", &mut entropy, &mut memory)
            .unwrap();
        // Password-only: the only credential cannot go, a PIN cannot change.
        assert_eq!(
            update(
                &mut store,
                Update::Remove,
                Kind::Password,
                b"2468",
                b"",
                0,
                &mut memory
            ),
            Err(Code::NotFound)
        );
        assert_eq!(
            update(
                &mut store,
                Update::Change,
                Kind::Pin,
                b"2468",
                b"1357",
                0,
                &mut memory
            ),
            Err(Code::NotFound)
        );
        // A new secret outside its policy is refused before any check.
        assert_eq!(
            update(
                &mut store,
                Update::Add,
                Kind::Pin,
                b"door-os!",
                b"12",
                0,
                &mut memory
            ),
            Err(Code::InvalidRequest)
        );
        assert_eq!(store.state(0).unwrap().failures, 0);
        // Add a PIN: the wrong password is rejected and changes nothing.
        assert_eq!(
            verdict(update(
                &mut store,
                Update::Add,
                Kind::Pin,
                b"wrong-pw",
                b"2468",
                0,
                &mut memory
            )),
            Ok(Verdict::Rejected {
                failures: 1,
                retry_after: 0
            })
        );
        assert!(!store.state(0).unwrap().both);
        let added = update(
            &mut store,
            Update::Add,
            Kind::Pin,
            b"door-os!",
            b"2468",
            0,
            &mut memory,
        )
        .unwrap();
        assert_eq!(added.verdict, Verdict::Accepted);
        // An accepted proof clears the shared failure count.
        assert_eq!(
            (added.state.enrolled, added.state.both, added.state.failures),
            (Some(Kind::Pin), true, 0)
        );
        assert_eq!(
            update(
                &mut store,
                Update::Add,
                Kind::Pin,
                b"door-os!",
                b"1357",
                0,
                &mut memory
            ),
            Err(Code::Exists)
        );
        // Change the PIN with the PIN (not with the password).
        assert!(matches!(
            verdict(update(
                &mut store,
                Update::Change,
                Kind::Pin,
                b"1111",
                b"1357",
                0,
                &mut memory
            )),
            Ok(Verdict::Rejected { failures: 1, .. })
        ));
        assert_eq!(
            verdict(update(
                &mut store,
                Update::Change,
                Kind::Pin,
                b"2468",
                b"1357",
                0,
                &mut memory
            )),
            Ok(Verdict::Accepted)
        );
        assert!(matches!(
            store.verify(Kind::Pin, b"2468", 0, &mut memory),
            Ok(Verdict::Rejected { .. })
        ));
        assert_eq!(
            store.verify(Kind::Pin, b"1357", 0, &mut memory),
            Ok(Verdict::Accepted)
        );
        // Change the password; the PIN is untouched.
        assert_eq!(
            verdict(update(
                &mut store,
                Update::Change,
                Kind::Password,
                b"door-os!",
                b"new pass",
                0,
                &mut memory
            )),
            Ok(Verdict::Accepted)
        );
        assert_eq!(
            store.verify(Kind::Password, b"new pass", 0, &mut memory),
            Ok(Verdict::Accepted)
        );
        // Remove the PIN with the password: the password remains.
        assert!(matches!(
            verdict(update(
                &mut store,
                Update::Remove,
                Kind::Pin,
                b"door-os!",
                b"",
                0,
                &mut memory
            )),
            Ok(Verdict::Rejected { .. })
        ));
        let removed = update(
            &mut store,
            Update::Remove,
            Kind::Pin,
            b"new pass",
            b"",
            0,
            &mut memory,
        )
        .unwrap();
        assert_eq!(removed.verdict, Verdict::Accepted);
        assert_eq!(
            (removed.state.enrolled, removed.state.both),
            (Some(Kind::Password), false)
        );
        assert_eq!(
            update(
                &mut store,
                Update::Remove,
                Kind::Password,
                b"1357",
                b"",
                0,
                &mut memory
            ),
            Err(Code::NotFound)
        );
        drop(store);
        // Each accepted update replaced the whole store once.
        assert_eq!(disk.saves, 5);
        let mut store = Store::new(&mut disk, TEST);
        let state = store.state(0).unwrap();
        assert_eq!((state.enrolled, state.both), (Some(Kind::Password), false));
        assert_eq!(
            store.verify(Kind::Pin, b"new pass", 0, &mut memory),
            Ok(Verdict::Accepted)
        );
        // A PIN-only account adds a password with its PIN, then removes it.
        let mut disk = Memory::default();
        let mut store = Store::new(&mut disk, TEST);
        store
            .enroll(Kind::Pin, b"2468", &mut entropy, &mut memory)
            .unwrap();
        assert_eq!(
            verdict(update(
                &mut store,
                Update::Add,
                Kind::Password,
                b"2468",
                b"door-os!",
                0,
                &mut memory
            )),
            Ok(Verdict::Accepted)
        );
        assert_eq!(
            verdict(update(
                &mut store,
                Update::Remove,
                Kind::Password,
                b"2468",
                b"",
                0,
                &mut memory
            )),
            Ok(Verdict::Accepted)
        );
        let state = store.state(0).unwrap();
        assert_eq!((state.enrolled, state.both), (Some(Kind::Pin), false));
    }

    #[test]
    fn throttle_is_shared_by_every_operation_that_checks_a_secret() {
        let mut memory = blocks(TEST);
        let mut entropy = counter();
        let mut disk = Memory::default();
        let mut store = Store::new(&mut disk, TEST);
        store
            .enroll(Kind::Pin, b"2468", &mut entropy, &mut memory)
            .unwrap();
        update(
            &mut store,
            Update::Add,
            Kind::Password,
            b"2468",
            b"door-os!",
            0,
            &mut memory,
        )
        .unwrap();
        // Wrong secrets through four different operations and both kinds.
        assert!(matches!(
            store.verify(Kind::Pin, b"1111", 5, &mut memory),
            Ok(Verdict::Rejected { failures: 1, .. })
        ));
        assert!(matches!(
            store.verify(Kind::Password, b"guess-me", 5, &mut memory),
            Ok(Verdict::Rejected { failures: 2, .. })
        ));
        assert!(matches!(
            verdict(update(
                &mut store,
                Update::Change,
                Kind::Pin,
                b"0000",
                b"1357",
                5,
                &mut memory
            )),
            Ok(Verdict::Rejected { failures: 3, .. })
        ));
        assert!(matches!(
            verdict(update(
                &mut store,
                Update::Remove,
                Kind::Pin,
                b"guess-me",
                b"",
                5,
                &mut memory
            )),
            Ok(Verdict::Rejected { failures: 4, .. })
        ));
        let fifth = update(
            &mut store,
            Update::Change,
            Kind::Password,
            b"guess-me",
            b"new pass",
            5,
            &mut memory,
        )
        .unwrap();
        assert_eq!(
            fifth.verdict,
            Verdict::Rejected {
                failures: credential::FREE_ATTEMPTS,
                retry_after: credential::FIRST_DELAY_TICKS
            }
        );
        assert_eq!(fifth.state.retry_after, credential::FIRST_DELAY_TICKS);
        // While throttled nothing is checked, through any operation.
        let saves = disk_saves(&store);
        for result in [
            verdict(update(
                &mut store,
                Update::Change,
                Kind::Pin,
                b"2468",
                b"1357",
                6,
                &mut memory,
            )),
            verdict(update(
                &mut store,
                Update::Remove,
                Kind::Password,
                b"2468",
                b"",
                6,
                &mut memory,
            )),
            store.verify(Kind::Pin, b"2468", 6, &mut memory),
        ] {
            assert!(matches!(result, Ok(Verdict::Throttled { failures: 5, .. })));
        }
        assert_eq!(disk_saves(&store), saves);
        assert_eq!(store.state(6).unwrap().failures, credential::FREE_ATTEMPTS);
        // After the delay the right PIN signs in and clears the count.
        let later = 5 + credential::FIRST_DELAY_TICKS;
        assert_eq!(
            store.verify(Kind::Pin, b"2468", later, &mut memory),
            Ok(Verdict::Accepted)
        );
        assert_eq!(store.state(later).unwrap().failures, 0);
    }
    fn disk_saves(store: &Store<&mut Memory>) -> usize {
        store.persist.saves
    }

    #[test]
    fn slow_proofs_skip_the_second_derivation_and_failed_writes_change_nothing() {
        let mut memory = blocks(TEST);
        let mut entropy = counter();
        let mut disk = Memory::default();
        let mut store = Store::new(&mut disk, TEST);
        store
            .enroll(Kind::Pin, b"2468", &mut entropy, &mut memory)
            .unwrap();
        // The proof took longer than the budget: retry, nothing changed.
        let mut slow = || 100 + SECOND_DERIVATION_BUDGET + 1;
        assert_eq!(
            store.update(
                Update::Change,
                Kind::Pin,
                b"2468",
                b"1357",
                100,
                &mut slow,
                &mut entropy,
                &mut memory,
            ),
            Err(Code::WouldBlock)
        );
        // Exactly at the budget the second derivation still runs.
        let mut on_time = || 100 + SECOND_DERIVATION_BUDGET;
        assert_eq!(
            verdict(store.update(
                Update::Change,
                Kind::Pin,
                b"2468",
                b"1357",
                100,
                &mut on_time,
                &mut entropy,
                &mut memory,
            )),
            Ok(Verdict::Accepted)
        );
        assert_eq!(
            store.verify(Kind::Pin, b"1357", 0, &mut memory),
            Ok(Verdict::Accepted)
        );
        // A failed write keeps the old credential in memory and reloads from
        // the store before the next request.
        store.persist.fail_save = true;
        assert_eq!(
            update(
                &mut store,
                Update::Change,
                Kind::Pin,
                b"1357",
                b"9999",
                0,
                &mut memory
            ),
            Err(Code::Io)
        );
        store.persist.fail_save = false;
        assert!(!store.loaded);
        assert_eq!(
            store.verify(Kind::Pin, b"1357", 0, &mut memory),
            Ok(Verdict::Accepted)
        );
        assert!(store.loaded);
        // ... including when the write did land before the error.
        let landed = Records {
            pin: Some(
                store
                    .fresh(Kind::Pin, b"8642", &mut entropy, &mut memory)
                    .unwrap(),
            ),
            password: None,
        };
        let mut bytes = [0; STORE_BYTES];
        let len = landed.encode(&mut bytes);
        store.persist.bytes = Some(bytes[..len].to_vec());
        store.loaded = false;
        assert_eq!(
            store.verify(Kind::Pin, b"8642", 0, &mut memory),
            Ok(Verdict::Accepted)
        );
    }

    #[test]
    fn damaged_or_foreign_records_fail_closed_and_unavailable_is_volatile() {
        let mut memory = blocks(TEST);
        let mut entropy = counter();
        let mut disk = Memory::default();
        Store::new(&mut disk, TEST)
            .enroll(Kind::Password, b"door-os!", &mut entropy, &mut memory)
            .unwrap();
        let good = disk.bytes.clone().unwrap();
        for index in [0, 8, 12, 20, 40, 60, 90] {
            let mut bad = good.clone();
            bad[index] ^= 0x40;
            disk.bytes = Some(bad);
            let mut store = Store::new(&mut disk, TEST);
            assert_eq!(store.state(0), Err(Code::Corrupt), "{index}");
            assert_eq!(
                store.enroll(Kind::Pin, b"0000", &mut entropy, &mut memory),
                Err(Code::Corrupt)
            );
            assert_eq!(
                store.verify(Kind::Password, b"door-os!", 0, &mut memory),
                Err(Code::Corrupt)
            );
            assert_eq!(
                update(
                    &mut store,
                    Update::Add,
                    Kind::Pin,
                    b"door-os!",
                    b"2468",
                    0,
                    &mut memory
                ),
                Err(Code::Corrupt)
            );
        }
        // A store too long to be any version fails closed as well.
        disk.bytes = Some(vec![0; STORE_BYTES + RECORD_BYTES]);
        assert_eq!(Store::new(&mut disk, TEST).state(0), Err(Code::Corrupt));
        // An empty store is a first enrollment interrupted before its write.
        disk.bytes = Some(Vec::new());
        {
            let mut store = Store::new(&mut disk, TEST);
            assert_eq!(store.state(0).unwrap().enrolled, None);
            store
                .enroll(Kind::Pin, b"2468", &mut entropy, &mut memory)
                .unwrap();
        }
        assert_eq!(disk.bytes.as_ref().map(Vec::len), Some(RECORD_BYTES));
        let mut volatile = Memory {
            unavailable: true,
            ..Memory::default()
        };
        {
            let mut store = Store::new(&mut volatile, TEST);
            assert!(!store.state(0).unwrap().persisted);
            store
                .enroll(Kind::Pin, b"9876", &mut entropy, &mut memory)
                .unwrap();
            assert_eq!(
                store.verify(Kind::Pin, b"9876", 0, &mut memory),
                Ok(Verdict::Accepted)
            );
            let added = update(
                &mut store,
                Update::Add,
                Kind::Password,
                b"9876",
                b"door-os!",
                0,
                &mut memory,
            )
            .unwrap();
            assert_eq!(
                (added.verdict, added.state.both, added.state.persisted),
                (Verdict::Accepted, true, false)
            );
        }
        assert_eq!(volatile.saves, 0);
        // A stuck entropy source never yields a salt.
        let mut stuck = |out: &mut [u8]| {
            out.fill(0);
            Ok(())
        };
        let mut fresh = Memory::default();
        assert_eq!(
            Store::new(&mut fresh, TEST).enroll(Kind::Pin, b"1234", &mut stuck, &mut memory),
            Err(Code::Resources)
        );
    }

    #[test]
    fn records_wipe_their_salt_and_tag_on_drop() {
        let mut memory = blocks(TEST);
        let mut record = Store::new(&mut Memory::default(), TEST)
            .fresh(Kind::Pin, b"2468", &mut counter(), &mut memory)
            .unwrap();
        assert!(record.tag.iter().any(|&b| b != 0));
        // What `Drop` runs, observed without reading freed memory.
        wipe(&mut record.salt);
        wipe(&mut record.tag);
        assert_eq!((record.salt, record.tag), ([0; SALT_BYTES], [0; TAG_BYTES]));
    }
}
