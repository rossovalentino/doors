#![no_std]
//! Original advapi32.dll: the registry (Reg*) over the storage service's
//! hives (HKEY_LOCAL_MACHINE, HKEY_CURRENT_USER, HKEY_CLASSES_ROOT with
//! their 32- and 64-bit views), and the token and SID functions over the
//! process's kernel-authenticated identity (user, groups, elevation and
//! integrity as Windows reports them for an administrator's limited or
//! full token).
//!
//! This module is the host-testable part: SIDs, the token information
//! records, registry handles, access masks and value checks.
//!
//! The same sources build the x64 and the x86 DLL (P17): Windows-facing
//! exports take pointer-width parameters, widened at entry; a pointer or
//! handle result is narrowed from the body's 64-bit value (an immediately
//! called closure, so the body's own `return`s keep their meaning).
#![allow(clippy::redundant_closure_call)]
extern crate alloc;
use alloc::{string::String, vec::Vec};

pub mod crypt;
#[cfg(all(feature = "guest-pe", target_os = "windows", not(test)))]
mod guest;
pub mod security;

/// Win32 error codes the functions return.
pub mod error {
    pub const SUCCESS: u32 = 0;
    pub const FILE_NOT_FOUND: u32 = 2;
    pub const ACCESS_DENIED: u32 = 5;
    pub const INVALID_HANDLE: u32 = 6;
    pub const NOT_ENOUGH_MEMORY: u32 = 8;
    pub const INVALID_DATA: u32 = 13;
    pub const NOT_SUPPORTED: u32 = 50;
    pub const INVALID_PARAMETER: u32 = 87;
    pub const CALL_NOT_IMPLEMENTED: u32 = 120;
    pub const INSUFFICIENT_BUFFER: u32 = 122;
    pub const BAD_PATHNAME: u32 = 161;
    pub const MORE_DATA: u32 = 234;
    pub const NO_MORE_ITEMS: u32 = 259;
    pub const NOT_ALL_ASSIGNED: u32 = 1300;
    pub const SOME_NOT_MAPPED: u32 = 1301;
    pub const NO_SUCH_PRIVILEGE: u32 = 1313;
    pub const INVALID_SID: u32 = 1337;
    pub const NONE_MAPPED: u32 = 1332;
    pub const NO_TOKEN: u32 = 1008;
    pub const KEY_DELETED: u32 = 1018;
    pub const CHILD_MUST_BE_VOLATILE: u32 = 1021;
    pub const CANTWRITE: u32 = 1013;
    pub const BADKEY: u32 = 1010;
    pub const NO_SUCH_LOGON_SESSION: u32 = 1312;
}

// ---- SIDs -------------------------------------------------------------------
/// A security identifier: authority and sub-authorities.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Sid {
    pub authority: u64,
    pub sub: Vec<u32>,
}
/// The machine part of local account SIDs (S-1-5-21-2026-26300-1-RID: three
/// sub-authorities, as every Windows machine domain has), as Terminal's
/// `whoami /user` reports it.
pub const MACHINE: [u32; 3] = [2026, 26300, 1];
impl Sid {
    pub fn new(authority: u64, sub: &[u32]) -> Self {
        Self {
            authority,
            sub: sub.into(),
        }
    }
    pub fn everyone() -> Self {
        Self::new(1, &[0])
    }
    pub fn local() -> Self {
        Self::new(2, &[0])
    }
    pub fn interactive() -> Self {
        Self::new(5, &[4])
    }
    pub fn authenticated() -> Self {
        Self::new(5, &[11])
    }
    pub fn this_organization() -> Self {
        Self::new(5, &[15])
    }
    pub fn local_system() -> Self {
        Self::new(5, &[18])
    }
    pub fn administrators() -> Self {
        Self::new(5, &[32, 544])
    }
    pub fn users() -> Self {
        Self::new(5, &[32, 545])
    }
    pub fn logon(session: u32) -> Self {
        Self::new(5, &[5, 0, session])
    }
    /// An account's SID: a local account's S-1-5-21-…-RID, the service
    /// account's LocalSystem (S-1-5-18).
    pub fn user(uid: u32) -> Self {
        if uid == door_abi::platform::SERVICE_ACCOUNT {
            return Self::local_system();
        }
        Self::new(5, &[21, MACHINE[0], MACHINE[1], MACHINE[2], uid])
    }
    /// Mandatory label: Low 0x1000, Medium 0x2000, High 0x3000, System 0x4000.
    pub fn integrity(level: u32) -> Self {
        Self::new(16, &[level])
    }
    pub fn length(&self) -> usize {
        8 + 4 * self.sub.len()
    }
    pub fn encode(&self) -> Vec<u8> {
        let mut b = Vec::with_capacity(self.length());
        b.push(1);
        b.push(self.sub.len() as u8);
        b.extend_from_slice(&self.authority.to_be_bytes()[2..]);
        for s in &self.sub {
            b.extend_from_slice(&s.to_le_bytes());
        }
        b
    }
    pub fn decode(b: &[u8]) -> Option<Self> {
        if b.len() < 8 || b[0] != 1 || b[1] > 15 || b.len() < 8 + 4 * usize::from(b[1]) {
            return None;
        }
        let mut authority = [0u8; 8];
        authority[2..].copy_from_slice(&b[2..8]);
        let sub = (0..usize::from(b[1]))
            .map(|i| u32::from_le_bytes(b[8 + 4 * i..12 + 4 * i].try_into().unwrap()))
            .collect();
        Some(Self {
            authority: u64::from_be_bytes(authority),
            sub,
        })
    }
    /// "S-1-5-32-544".
    pub fn text(&self) -> String {
        let mut s = alloc::format!("S-1-{}", self.authority);
        for a in &self.sub {
            s.push_str(&alloc::format!("-{a}"));
        }
        s
    }
    pub fn parse(text: &str) -> Option<Self> {
        let mut parts = text
            .strip_prefix("S-1-")
            .or_else(|| text.strip_prefix("s-1-"))?
            .split('-');
        let authority = parts.next()?.parse().ok()?;
        let sub: Vec<u32> = parts.map(|p| p.parse().ok()).collect::<Option<_>>()?;
        (sub.len() <= 15).then_some(Self { authority, sub })
    }
    /// (domain, name, SID_NAME_USE) for `sid`; `user` names the caller's
    /// own account (uid, name).
    pub fn account(&self, user: (u32, &str)) -> Option<(String, String, u32)> {
        security::account(self, user)
    }
}

// ---- the token ------------------------------------------------------------------
/// What Doors records of a process's token (from its kernel identity).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Token {
    pub uid: u32,
    pub session: u32,
    /// BUILTIN\Administrators enabled: the full token.
    pub elevated: bool,
    /// The account is an administrator: unelevated, its token is the
    /// filtered one (Administrators for deny only, a linked full token).
    pub administrator: bool,
    /// TOKEN_TYPE: 1 primary, 2 impersonation.
    pub kind: u8,
    /// SECURITY_IMPERSONATION_LEVEL of an impersonation token (0..=3).
    pub level: u8,
}
/// SE_GROUP_* attributes.
pub mod attr {
    pub const MANDATORY: u32 = 0x1;
    pub const ENABLED_BY_DEFAULT: u32 = 0x2;
    pub const ENABLED: u32 = 0x4;
    pub const OWNER: u32 = 0x8;
    pub const USE_FOR_DENY_ONLY: u32 = 0x10;
    pub const INTEGRITY: u32 = 0x20;
    pub const INTEGRITY_ENABLED: u32 = 0x40;
    pub const LOGON_ID: u32 = 0xC000_0000;
    pub const STANDARD: u32 = MANDATORY | ENABLED_BY_DEFAULT | ENABLED;
}
/// Privileges by LUID (Windows' fixed values) and name.
pub const PRIVILEGES: &[(u32, &str)] = &[
    (2, "SeCreateTokenPrivilege"),
    (3, "SeAssignPrimaryTokenPrivilege"),
    (4, "SeLockMemoryPrivilege"),
    (5, "SeIncreaseQuotaPrivilege"),
    (6, "SeMachineAccountPrivilege"),
    (7, "SeTcbPrivilege"),
    (8, "SeSecurityPrivilege"),
    (9, "SeTakeOwnershipPrivilege"),
    (10, "SeLoadDriverPrivilege"),
    (11, "SeSystemProfilePrivilege"),
    (12, "SeSystemtimePrivilege"),
    (13, "SeProfileSingleProcessPrivilege"),
    (14, "SeIncreaseBasePriorityPrivilege"),
    (15, "SeCreatePagefilePrivilege"),
    (16, "SeCreatePermanentPrivilege"),
    (17, "SeBackupPrivilege"),
    (18, "SeRestorePrivilege"),
    (19, "SeShutdownPrivilege"),
    (20, "SeDebugPrivilege"),
    (21, "SeAuditPrivilege"),
    (22, "SeSystemEnvironmentPrivilege"),
    (23, "SeChangeNotifyPrivilege"),
    (24, "SeRemoteShutdownPrivilege"),
    (25, "SeUndockPrivilege"),
    (26, "SeSyncAgentPrivilege"),
    (27, "SeEnableDelegationPrivilege"),
    (28, "SeManageVolumePrivilege"),
    (29, "SeImpersonatePrivilege"),
    (30, "SeCreateGlobalPrivilege"),
    (31, "SeTrustedCredManAccessPrivilege"),
    (32, "SeRelabelPrivilege"),
    (33, "SeIncreaseWorkingSetPrivilege"),
    (34, "SeTimeZonePrivilege"),
    (35, "SeCreateSymbolicLinkPrivilege"),
    (36, "SeDelegateSessionUserImpersonatePrivilege"),
];
pub fn privilege_luid(name: &str) -> Option<u32> {
    PRIVILEGES
        .iter()
        .find(|(_, n)| n.eq_ignore_ascii_case(name))
        .map(|(l, _)| *l)
}
pub fn privilege_name(luid: u32) -> Option<&'static str> {
    PRIVILEGES.iter().find(|(l, _)| *l == luid).map(|(_, n)| *n)
}
/// SE_PRIVILEGE_ENABLED_BY_DEFAULT | ENABLED.
pub const PRIVILEGE_ON: u32 = 0x3;
pub const PRIVILEGE_ENABLED: u32 = 0x2;
impl Token {
    /// The process token of a session application.
    pub fn primary(uid: u32, session: u32, elevated: bool, administrator: bool) -> Self {
        Self {
            uid,
            session,
            elevated,
            administrator: administrator || elevated,
            kind: 1,
            level: 0,
        }
    }
    /// LocalSystem (the service account) holds the System level.
    fn system(&self) -> bool {
        self.uid == door_abi::platform::SERVICE_ACCOUNT
    }
    pub fn integrity(&self) -> u32 {
        if self.system() {
            0x4000
        } else if self.elevated {
            0x3000
        } else {
            0x2000
        }
    }
    /// TOKEN_ELEVATION_TYPE: Default (1) for a standard user, Full (2)
    /// elevated, Limited (3) for an administrator's filtered token.
    pub fn elevation_type(&self) -> u32 {
        // LocalSystem's token is elevated with no linked token: Default.
        if self.system() {
            1
        } else if self.elevated {
            2
        } else if self.administrator {
            3
        } else {
            1
        }
    }
    /// The token's groups in Windows' order (measured on a local account):
    /// the integrity label first, then Everyone, the administrator groups an
    /// administrator's token carries, Users and the logon groups.
    pub fn groups(&self) -> Vec<(Sid, u32)> {
        // LocalSystem's: Administrators (its owner), Everyone, Authenticated
        // Users and the System label, as `whoami /groups` lists them there.
        if self.system() {
            return alloc::vec![
                (Sid::administrators(), attr::STANDARD | attr::OWNER),
                (Sid::everyone(), attr::STANDARD),
                (Sid::authenticated(), attr::STANDARD),
                (
                    Sid::integrity(0x4000),
                    attr::INTEGRITY | attr::INTEGRITY_ENABLED
                ),
            ];
        }
        let mut g = alloc::vec![
            (
                Sid::integrity(self.integrity()),
                attr::INTEGRITY | attr::INTEGRITY_ENABLED
            ),
            (Sid::everyone(), attr::STANDARD),
        ];
        if self.administrator {
            let (local_admin, admins) = if self.elevated {
                (attr::STANDARD, attr::STANDARD | attr::OWNER)
            } else {
                (attr::USE_FOR_DENY_ONLY, attr::USE_FOR_DENY_ONLY)
            };
            g.push((Sid::new(5, &[114]), local_admin));
            g.push((Sid::administrators(), admins));
        }
        g.extend([
            (Sid::users(), attr::STANDARD),
            (Sid::interactive(), attr::STANDARD),
            (Sid::new(2, &[1]), attr::STANDARD),
            (Sid::authenticated(), attr::STANDARD),
            (Sid::this_organization(), attr::STANDARD),
            (Sid::new(5, &[113]), attr::STANDARD),
            (Sid::logon(self.session), attr::STANDARD | attr::LOGON_ID),
            (Sid::local(), attr::STANDARD),
            (Sid::new(5, &[64, 10]), attr::STANDARD),
        ]);
        g
    }
    /// Privileges (LUID, attributes): the limited token keeps five, the full
    /// token an administrator's set (most disabled until adjusted).
    pub fn privileges(&self) -> Vec<(u32, u32)> {
        if !self.elevated {
            return alloc::vec![(19, 0), (23, PRIVILEGE_ON), (25, 0), (33, 0), (34, 0)];
        }
        [
            5, 8, 9, 10, 11, 12, 13, 14, 15, 17, 18, 19, 20, 22, 23, 24, 25, 28, 29, 30, 33, 34,
            35, 36,
        ]
        .iter()
        .map(|&l| {
            (
                l,
                if matches!(l, 23 | 29 | 30) {
                    PRIVILEGE_ON
                } else {
                    0
                },
            )
        })
        .collect()
    }
    /// Is `sid` an enabled group (CheckTokenMembership)?
    pub fn member(&self, sid: &Sid) -> bool {
        *sid == Sid::user(self.uid)
            || self.groups().iter().any(|(g, a)| {
                g == sid && a & attr::ENABLED != 0 && a & attr::USE_FOR_DENY_ONLY == 0
            })
    }
    /// The token's owner for new objects: Administrators when elevated.
    pub fn owner(&self) -> Sid {
        if self.elevated {
            Sid::administrators()
        } else {
            Sid::user(self.uid)
        }
    }
    /// TokenDefaultDacl: what new objects get (the reference's filtered
    /// token: the user, Administrators and SYSTEM, GENERIC_ALL).
    pub fn default_dacl(&self) -> security::Acl {
        use security::{Ace, ace};
        let mut aces = alloc::vec![Ace::new(ace::ALLOWED, 0, 0x1000_0000, Sid::user(self.uid))];
        if self.administrator {
            aces.push(Ace::new(
                ace::ALLOWED,
                0,
                0x1000_0000,
                Sid::administrators(),
            ));
        }
        aces.push(Ace::new(ace::ALLOWED, 0, 0x1000_0000, Sid::local_system()));
        if self.elevated {
            aces.remove(0);
        }
        security::Acl { aces }
    }
}

/// TOKEN_INFORMATION_CLASS values Doors answers.
pub mod class {
    pub const USER: u32 = 1;
    pub const GROUPS: u32 = 2;
    pub const PRIVILEGES: u32 = 3;
    pub const OWNER: u32 = 4;
    pub const PRIMARY_GROUP: u32 = 5;
    pub const DEFAULT_DACL: u32 = 6;
    pub const TYPE: u32 = 8;
    pub const IMPERSONATION_LEVEL: u32 = 9;
    pub const STATISTICS: u32 = 10;
    pub const SESSION_ID: u32 = 12;
    pub const ELEVATION_TYPE: u32 = 18;
    pub const LINKED_TOKEN: u32 = 19;
    pub const ELEVATION: u32 = 20;
    pub const HAS_RESTRICTIONS: u32 = 21;
    pub const VIRTUALIZATION_ALLOWED: u32 = 23;
    pub const VIRTUALIZATION_ENABLED: u32 = 24;
    pub const INTEGRITY_LEVEL: u32 = 25;
    pub const UI_ACCESS: u32 = 26;
    pub const MANDATORY_POLICY: u32 = 27;
    pub const LOGON_SID: u32 = 28;
    pub const IS_APP_CONTAINER: u32 = 29;
}
/// A TOKEN_PRIVILEGES record: the count, then (LUID, attributes) each.
pub fn privileges_record(p: &[(u32, u32)]) -> Vec<u8> {
    let mut b = Vec::with_capacity(4 + 12 * p.len());
    b.extend_from_slice(&(p.len() as u32).to_le_bytes());
    for (luid, a) in p {
        b.extend_from_slice(&luid.to_le_bytes());
        b.extend_from_slice(&0u32.to_le_bytes());
        b.extend_from_slice(&a.to_le_bytes());
    }
    b
}
/// A GetTokenInformation record for a buffer at address `base` in a
/// program whose pointers are `width` bytes (8 x64, 4 x86, P17): its bytes
/// (SID_AND_ATTRIBUTES pointers point into the same buffer). Err for a
/// class Doors does not answer (ERROR_INVALID_PARAMETER) or one the token
/// has no value for.
pub fn token_information(
    token: &Token,
    class: u32,
    base: u64,
    width: usize,
) -> Result<Vec<u8>, u32> {
    let mut b = Vec::new();
    let pointer = |value: u64| value.to_le_bytes()[..width].to_vec();
    // SID_AND_ATTRIBUTES (PSID, DWORD; a pointer's alignment) arrays
    // followed by the SIDs.
    let entry = 2 * width;
    let with_sids = |b: &mut Vec<u8>, entries: &[(Sid, u32)]| {
        let table = b.len();
        b.resize(table + entries.len() * entry, 0);
        for (i, (sid, a)) in entries.iter().enumerate() {
            let at = b.len();
            b.extend_from_slice(&sid.encode());
            let slot = table + i * entry;
            b[slot..slot + width].copy_from_slice(&pointer(base + at as u64));
            b[slot + width..slot + width + 4].copy_from_slice(&a.to_le_bytes());
        }
    };
    // TOKEN_GROUPS: the count, then the array at a pointer's alignment.
    let groups_record = |b: &mut Vec<u8>, groups: &[(Sid, u32)]| {
        b.extend_from_slice(&(groups.len() as u32).to_le_bytes());
        b.resize(width.max(4), 0);
        with_sids(b, groups);
    };
    match class {
        class::USER => with_sids(&mut b, &[(Sid::user(token.uid), 0)]),
        class::GROUPS => groups_record(&mut b, &token.groups()),
        class::LOGON_SID => groups_record(
            &mut b,
            &[(Sid::logon(token.session), attr::STANDARD | attr::LOGON_ID)],
        ),
        class::PRIVILEGES => b = privileges_record(&token.privileges()),
        class::OWNER | class::PRIMARY_GROUP => {
            // TOKEN_OWNER / TOKEN_PRIMARY_GROUP: one pointer, then the SID
            // (a local account's primary group is the account itself, as the
            // reference reports).
            let sid = if class == class::OWNER {
                token.owner()
            } else {
                Sid::user(token.uid)
            };
            b.extend_from_slice(&pointer(base + width as u64));
            b.extend_from_slice(&sid.encode());
        }
        class::DEFAULT_DACL => {
            b.extend_from_slice(&pointer(base + width as u64));
            b.extend_from_slice(&token.default_dacl().encode());
        }
        class::TYPE => b.extend_from_slice(&u32::from(token.kind).to_le_bytes()),
        class::IMPERSONATION_LEVEL => {
            if token.kind != 2 {
                return Err(error::INVALID_PARAMETER);
            }
            b.extend_from_slice(&u32::from(token.level).to_le_bytes());
        }
        class::SESSION_ID => b.extend_from_slice(&token.session.to_le_bytes()),
        class::ELEVATION_TYPE => b.extend_from_slice(&token.elevation_type().to_le_bytes()),
        class::ELEVATION => b.extend_from_slice(&u32::from(token.elevated).to_le_bytes()),
        class::HAS_RESTRICTIONS => {
            b.extend_from_slice(&u32::from(token.administrator && !token.elevated).to_le_bytes())
        }
        class::UI_ACCESS
        | class::IS_APP_CONTAINER
        | class::VIRTUALIZATION_ALLOWED
        | class::VIRTUALIZATION_ENABLED => b.extend_from_slice(&0u32.to_le_bytes()),
        // TOKEN_MANDATORY_POLICY_NO_WRITE_UP | NEW_PROCESS_MIN.
        class::MANDATORY_POLICY => b.extend_from_slice(&3u32.to_le_bytes()),
        class::INTEGRITY_LEVEL => with_sids(
            &mut b,
            &[(
                Sid::integrity(token.integrity()),
                attr::INTEGRITY | attr::INTEGRITY_ENABLED,
            )],
        ),
        class::STATISTICS => {
            // TOKEN_STATISTICS (56 bytes): token id, authentication id,
            // expiration, type, impersonation level, dynamic charged and
            // available, counts, modified id.
            b.resize(56, 0);
            b[0..4]
                .copy_from_slice(&(0x1000 + token.uid + u32::from(token.elevated)).to_le_bytes());
            b[8..12].copy_from_slice(&(0x3E7 + token.session).to_le_bytes());
            b[16..24].copy_from_slice(&i64::MAX.to_le_bytes());
            b[24..28].copy_from_slice(&u32::from(token.kind).to_le_bytes());
            b[28..32].copy_from_slice(&u32::from(token.level).to_le_bytes());
            let dacl = token.default_dacl().length() as u32;
            b[32..36].copy_from_slice(&4096u32.to_le_bytes());
            b[36..40].copy_from_slice(&(4096 - dacl).to_le_bytes());
            b[40..44].copy_from_slice(&(token.groups().len() as u32).to_le_bytes());
            b[44..48].copy_from_slice(&(token.privileges().len() as u32).to_le_bytes());
            b[48..52]
                .copy_from_slice(&(0x2000 + token.uid + u32::from(token.elevated)).to_le_bytes());
        }
        _ => return Err(error::INVALID_PARAMETER),
    }
    Ok(b)
}

// ---- the registry -----------------------------------------------------------------
pub mod hkey {
    pub const CLASSES_ROOT: u64 = 0x8000_0000;
    pub const CURRENT_USER: u64 = 0x8000_0001;
    pub const LOCAL_MACHINE: u64 = 0x8000_0002;
    pub const USERS: u64 = 0x8000_0003;
    pub const PERFORMANCE_DATA: u64 = 0x8000_0004;
    pub const CURRENT_CONFIG: u64 = 0x8000_0005;
}
/// REGSAM bits.
pub mod sam {
    pub const QUERY_VALUE: u32 = 0x1;
    pub const SET_VALUE: u32 = 0x2;
    pub const CREATE_SUB_KEY: u32 = 0x4;
    pub const ENUMERATE_SUB_KEYS: u32 = 0x8;
    pub const NOTIFY: u32 = 0x10;
    pub const CREATE_LINK: u32 = 0x20;
    pub const WOW64_64KEY: u32 = 0x100;
    pub const WOW64_32KEY: u32 = 0x200;
    pub const DELETE: u32 = 0x1_0000;
    pub const MAXIMUM_ALLOWED: u32 = 0x200_0000;
    pub const GENERIC_READ: u32 = 0x8000_0000;
    pub const GENERIC_WRITE: u32 = 0x4000_0000;
    pub const GENERIC_ALL: u32 = 0x1000_0000;
}
/// The storage access a REGSAM needs: 1 read, 2 write (values and
/// subkeys), 4 delete.
pub fn storage_access(desired: u32) -> u16 {
    let mut access = 1;
    if desired
        & (sam::SET_VALUE
            | sam::CREATE_SUB_KEY
            | sam::CREATE_LINK
            | sam::GENERIC_WRITE
            | sam::GENERIC_ALL)
        != 0
        || desired & 0x000F_003F == 0x000F_003F
    {
        access |= 2;
    }
    if desired & (sam::DELETE | sam::GENERIC_ALL) != 0 {
        access |= 4;
    }
    access
}
/// Where a predefined key lives: (storage root 0 HKLM / 1 HKCU / 2 HKCR,
/// path prefix). HKEY_USERS reaches the caller's own hive (its SID or
/// .DEFAULT) only; HKEY_CURRENT_CONFIG is HKLM's current hardware profile.
pub fn predefined(key: u64, user_sid: &str, rest: &[u16]) -> Option<(u8, Vec<u16>)> {
    let path = |prefix: &str| {
        let mut p: Vec<u16> = prefix.encode_utf16().collect();
        if !rest.is_empty() {
            if !p.is_empty() {
                p.push(u16::from(b'\\'));
            }
            p.extend_from_slice(rest);
        }
        p
    };
    Some(match key {
        hkey::LOCAL_MACHINE => (0, path("")),
        hkey::CURRENT_USER => (1, path("")),
        hkey::CLASSES_ROOT => (2, path("")),
        hkey::CURRENT_CONFIG => (
            0,
            path("SYSTEM\\CurrentControlSet\\Hardware Profiles\\Current"),
        ),
        hkey::USERS => {
            let text: String = String::from_utf16_lossy(rest);
            let (first, tail) = text.split_once('\\').unwrap_or((text.as_str(), ""));
            if !(first.eq_ignore_ascii_case(user_sid) || first.eq_ignore_ascii_case(".DEFAULT")) {
                return None;
            }
            (1, tail.encode_utf16().collect())
        }
        _ => return None,
    })
}
/// A subkey path joined onto an open key's path, normalised: separators
/// collapse, a leading separator is dropped. None for an empty segment
/// sequence that names "..", which Windows does not resolve.
pub fn join(base: &[u16], sub: &[u16]) -> Vec<u16> {
    let mut out: Vec<u16> = base.to_vec();
    for part in sub
        .split(|u| *u == u16::from(b'\\'))
        .filter(|p| !p.is_empty())
    {
        if !out.is_empty() {
            out.push(u16::from(b'\\'));
        }
        out.extend_from_slice(part);
    }
    out
}
/// REG_* value types the hives store.
pub mod kind {
    pub const NONE: u32 = 0;
    pub const SZ: u32 = 1;
    pub const EXPAND_SZ: u32 = 2;
    pub const BINARY: u32 = 3;
    pub const DWORD: u32 = 4;
    pub const DWORD_BIG_ENDIAN: u32 = 5;
    pub const MULTI_SZ: u32 = 7;
    pub const QWORD: u32 = 11;
}
/// A value the caller hands RegSetValueEx, as the hive will store it:
/// strings get their terminator (Windows stores what it is given, but
/// readers expect one), DWORD/QWORD must have their exact sizes.
pub fn stored(kind: u32, data: &[u8]) -> Result<Vec<u8>, u32> {
    let mut d = data.to_vec();
    match kind {
        kind::DWORD if d.len() != 4 => return Err(error::INVALID_PARAMETER),
        kind::QWORD if d.len() != 8 => return Err(error::INVALID_PARAMETER),
        kind::SZ | kind::EXPAND_SZ => {
            if d.len() % 2 == 1 {
                d.push(0);
            }
            if !d.ends_with(&[0, 0]) {
                d.extend_from_slice(&[0, 0]);
            }
        }
        kind::MULTI_SZ => {
            if d.len() % 2 == 1 {
                d.push(0);
            }
            while !d.ends_with(&[0, 0, 0, 0]) {
                d.extend_from_slice(&[0, 0]);
            }
        }
        kind::NONE | kind::BINARY | kind::DWORD | kind::QWORD => (),
        kind::DWORD_BIG_ENDIAN => return Err(error::NOT_SUPPORTED),
        _ => return Err(error::NOT_SUPPORTED),
    }
    Ok(d)
}
/// UTF-16 string data (with terminators) as the ANSI functions return it,
/// and back.
pub fn wide_to_ansi(data: &[u8]) -> Vec<u8> {
    data.chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .map(|u| if u < 0x100 { u as u8 } else { b'?' })
        .collect()
}
pub fn ansi_to_wide(data: &[u8]) -> Vec<u8> {
    data.iter()
        .flat_map(|b| (u16::from(*b)).to_le_bytes())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;
    #[test]
    fn sids_encode_parse_and_name() {
        let a = Sid::administrators();
        assert_eq!(
            a.encode(),
            vec![1, 2, 0, 0, 0, 0, 0, 5, 32, 0, 0, 0, 0x20, 2, 0, 0]
        );
        assert_eq!(Sid::decode(&a.encode()), Some(a.clone()));
        assert_eq!(a.text(), "S-1-5-32-544");
        assert_eq!(Sid::parse("S-1-5-32-544"), Some(a.clone()));
        assert_eq!(Sid::parse("S-1-x"), None);
        assert_eq!(a.account((1001, "u")).unwrap().1, "Administrators");
        let me = Sid::user(1001);
        assert_eq!(
            me.account((1001, "Ada")),
            Some((String::from("DOORS"), String::from("Ada"), 1))
        );
        assert_eq!(me.text(), "S-1-5-21-2026-26300-1-1001");
    }
    #[test]
    fn the_service_account_is_local_system() {
        let system = Token::primary(door_abi::platform::SERVICE_ACCOUNT, 2, true, true);
        assert_eq!(
            Sid::user(door_abi::platform::SERVICE_ACCOUNT).text(),
            "S-1-5-18"
        );
        assert_eq!(system.integrity(), 0x4000);
        assert_eq!(system.elevation_type(), 1);
        assert!(system.member(&Sid::administrators()));
        assert!(system.member(&Sid::local_system()));
        assert!(!system.member(&Sid::users()) && !system.member(&Sid::interactive()));
        assert_eq!(system.owner(), Sid::administrators());
    }
    #[test]
    fn standard_limited_and_full_tokens() {
        let limited = Token::primary(1001, 1, false, true);
        let full = Token::primary(1001, 1, true, true);
        let standard = Token::primary(1002, 1, false, false);
        assert_eq!(
            [
                standard.elevation_type(),
                full.elevation_type(),
                limited.elevation_type()
            ],
            [1, 2, 3]
        );
        // A standard user's token has no Administrators group at all.
        assert!(
            !standard
                .groups()
                .iter()
                .any(|(g, _)| *g == Sid::administrators() || *g == Sid::new(5, &[114]))
        );
        assert_eq!(standard.groups().len(), 11);
        assert_eq!(limited.groups().len(), 13);
        assert_eq!(limited.groups()[0].0, Sid::integrity(0x2000));
        assert_eq!(
            token_information(&standard, class::HAS_RESTRICTIONS, 0, 8),
            Ok(0u32.to_le_bytes().to_vec())
        );
        assert_eq!(
            token_information(&limited, class::IMPERSONATION_LEVEL, 0, 8),
            Err(error::INVALID_PARAMETER)
        );
        let impersonation = Token {
            kind: 2,
            level: 2,
            ..limited
        };
        assert_eq!(
            token_information(&impersonation, class::IMPERSONATION_LEVEL, 0, 8),
            Ok(2u32.to_le_bytes().to_vec())
        );
        let dacl = token_information(&limited, class::DEFAULT_DACL, 0x1000, 8).unwrap();
        let acl = security::Acl::decode(&dacl[8..]).unwrap();
        assert_eq!(acl.aces.len(), 3);
        assert_eq!(acl.aces[1].sid, Sid::administrators());
        assert!(!limited.member(&Sid::administrators()) && full.member(&Sid::administrators()));
        assert!(limited.member(&Sid::users()) && limited.member(&Sid::user(1001)));
        assert_eq!(limited.integrity(), 0x2000);
        assert_eq!(full.integrity(), 0x3000);
        assert_eq!(limited.privileges().len(), 5);
        assert!(full.privileges().iter().any(|(l, _)| *l == 20));
        let base = 0x10_0000;
        let elev = token_information(&full, class::ELEVATION_TYPE, base, 8).unwrap();
        assert_eq!(elev, 2u32.to_le_bytes());
        let label = token_information(&limited, class::INTEGRITY_LEVEL, base, 8).unwrap();
        assert_eq!(
            u64::from_le_bytes(label[..8].try_into().unwrap()),
            base + 16
        );
        assert_eq!(Sid::decode(&label[16..]), Some(Sid::integrity(0x2000)));
        let groups = token_information(&limited, class::GROUPS, base, 8).unwrap();
        let count = u32::from_le_bytes(groups[..4].try_into().unwrap()) as usize;
        assert_eq!(count, limited.groups().len());
        let first = u64::from_le_bytes(groups[8..16].try_into().unwrap()) - base;
        assert_eq!(
            Sid::decode(&groups[first as usize..]),
            Some(Sid::integrity(0x2000))
        );
        // x86 (P17): 8-byte SID_AND_ATTRIBUTES right after the count.
        let groups32 = token_information(&limited, class::GROUPS, base, 4).unwrap();
        let first = u64::from(u32::from_le_bytes(groups32[4..8].try_into().unwrap())) - base;
        assert_eq!(first as usize, 4 + 8 * count);
        assert_eq!(
            Sid::decode(&groups32[first as usize..]),
            Some(Sid::integrity(0x2000))
        );
        let label32 = token_information(&limited, class::INTEGRITY_LEVEL, base, 4).unwrap();
        assert_eq!(
            u32::from_le_bytes(label32[..4].try_into().unwrap()) as u64,
            base + 8
        );
        let owner32 = token_information(&limited, class::OWNER, base, 4).unwrap();
        assert_eq!(
            u32::from_le_bytes(owner32[..4].try_into().unwrap()) as u64,
            base + 4
        );
        assert_eq!(privilege_luid("sedebugprivilege"), Some(20));
        assert_eq!(privilege_name(23), Some("SeChangeNotifyPrivilege"));
    }
    #[test]
    fn registry_paths_access_and_values() {
        let w = |s: &str| -> Vec<u16> { s.encode_utf16().collect() };
        assert_eq!(
            predefined(hkey::LOCAL_MACHINE, "S-1-5-21-1", &w("SOFTWARE\\Doors")),
            Some((0, w("SOFTWARE\\Doors")))
        );
        assert_eq!(
            predefined(hkey::USERS, "S-1-5-21-1", &w("S-1-5-21-1\\Environment")),
            Some((1, w("Environment")))
        );
        assert_eq!(
            predefined(hkey::USERS, "S-1-5-21-1", &w("S-1-5-21-9")),
            None
        );
        assert_eq!(predefined(hkey::CURRENT_CONFIG, "x", &[]).unwrap().0, 0);
        assert_eq!(
            join(&w("SOFTWARE"), &w("\\Doors\\\\Test\\")),
            w("SOFTWARE\\Doors\\Test")
        );
        assert_eq!(storage_access(0x2_0019), 1);
        assert_eq!(storage_access(0xF_003F), 1 | 2 | 4);
        assert_eq!(storage_access(sam::SET_VALUE), 3);
        assert_eq!(
            stored(
                kind::SZ,
                &w("ab")
                    .iter()
                    .flat_map(|u| u.to_le_bytes())
                    .collect::<Vec<_>>()
            )
            .unwrap()
            .len(),
            6
        );
        assert_eq!(
            stored(kind::DWORD, &[1, 2, 3]),
            Err(error::INVALID_PARAMETER)
        );
        // Correctly sized numbers are stored as given.
        assert_eq!(stored(kind::DWORD, &[1, 2, 3, 4]), Ok(vec![1, 2, 3, 4]));
        assert_eq!(stored(kind::QWORD, &[7; 8]), Ok(vec![7; 8]));
        assert_eq!(
            stored(kind::DWORD_BIG_ENDIAN, &[0; 4]),
            Err(error::NOT_SUPPORTED)
        );
        assert_eq!(
            stored(kind::MULTI_SZ, &[b'a', 0]).unwrap(),
            vec![b'a', 0, 0, 0, 0, 0]
        );
        assert_eq!(wide_to_ansi(&ansi_to_wide(b"x\0")), b"x\0");
    }
}
