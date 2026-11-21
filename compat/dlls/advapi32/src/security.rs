//! Security descriptors, ACLs, SDDL, well-known SIDs and account names, the
//! access check, and Doors' storage ACLs as Windows descriptors.
//!
//! Tables and formats were measured on Windows 11 26H2 (secprobe.exe dump):
//! CreateWellKnownSid, LookupAccountSid, the SDDL SID aliases and rights
//! strings, the self-relative layout ConvertStringSecurityDescriptor
//! produces (header, SACL, DACL, owner, group).
use crate::{MACHINE, Sid, error};
use alloc::{format, string::String, vec::Vec};

/// Win32 errors of the security functions.
pub mod code {
    pub const BAD_LENGTH: u32 = 24;
    pub const UNKNOWN_REVISION: u32 = 1305;
    pub const INVALID_OWNER: u32 = 1307;
    pub const INVALID_PRIMARY_GROUP: u32 = 1308;
    pub const NO_IMPERSONATION_TOKEN: u32 = 1309;
    pub const INVALID_ACL: u32 = 1336;
    pub const INVALID_SECURITY_DESCR: u32 = 1338;
    pub const ALLOTTED_SPACE_EXCEEDED: u32 = 1344;
    pub const BAD_IMPERSONATION_LEVEL: u32 = 1346;
    pub const INVALID_DATATYPE: u32 = 1804;
    pub const INVALID_FLAGS: u32 = 1004;
    pub const GENERIC_NOT_MAPPED: u32 = 1360;
    pub const BAD_DESCRIPTOR_FORMAT: u32 = 1361;
    pub const BAD_TOKEN_TYPE: u32 = 1349;
    pub const PRIVILEGE_NOT_HELD: u32 = 1314;
}

// ---- well-known SIDs and names ----------------------------------------------------
/// CreateWellKnownSid without a domain (WELL_KNOWN_SID_TYPE, authority,
/// sub-authorities); the types that need a domain answer
/// ERROR_INVALID_PARAMETER.
const WELL_KNOWN: &[(u32, u64, &[u32])] = &[
    (0, 0, &[0]),
    (1, 1, &[0]),
    (2, 2, &[0]),
    (3, 3, &[0]),
    (4, 3, &[1]),
    (5, 3, &[2]),
    (6, 3, &[3]),
    (7, 5, &[]),
    (8, 5, &[1]),
    (9, 5, &[2]),
    (10, 5, &[3]),
    (11, 5, &[4]),
    (12, 5, &[6]),
    (13, 5, &[7]),
    (14, 5, &[8]),
    (15, 5, &[9]),
    (16, 5, &[10]),
    (17, 5, &[11]),
    (18, 5, &[12]),
    (19, 5, &[13]),
    (20, 5, &[14]),
    (22, 5, &[18]),
    (23, 5, &[19]),
    (24, 5, &[20]),
    (25, 5, &[32]),
    (26, 5, &[32, 544]),
    (27, 5, &[32, 545]),
    (28, 5, &[32, 546]),
    (29, 5, &[32, 547]),
    (30, 5, &[32, 548]),
    (31, 5, &[32, 549]),
    (32, 5, &[32, 550]),
    (33, 5, &[32, 551]),
    (34, 5, &[32, 552]),
    (35, 5, &[32, 554]),
    (36, 5, &[32, 555]),
    (37, 5, &[32, 556]),
    (51, 5, &[64, 10]),
    (52, 5, &[64, 21]),
    (53, 5, &[64, 14]),
    (54, 5, &[15]),
    (55, 5, &[1000]),
    (56, 5, &[32, 557]),
    (57, 5, &[32, 558]),
    (58, 5, &[32, 559]),
    (59, 5, &[32, 560]),
    (60, 5, &[32, 561]),
    (61, 5, &[32, 562]),
    (62, 5, &[32, 568]),
    (63, 5, &[17]),
    (64, 5, &[32, 569]),
    (65, 16, &[0]),
    (66, 16, &[4096]),
    (67, 16, &[8192]),
    (68, 16, &[12288]),
    (69, 16, &[16384]),
    (70, 5, &[33]),
    (71, 3, &[4]),
    (74, 5, &[22]),
    (76, 5, &[32, 573]),
    (78, 5, &[32, 574]),
    (79, 16, &[8448]),
    (81, 2, &[1]),
    (82, 5, &[65, 1]),
    (84, 15, &[2, 1]),
    (85, 15, &[3, 1]),
    (86, 15, &[3, 2]),
    (87, 15, &[3, 3]),
    (88, 15, &[3, 4]),
    (89, 15, &[3, 5]),
    (90, 15, &[3, 6]),
    (91, 15, &[3, 7]),
    (92, 15, &[3, 9]),
    (93, 15, &[3, 8]),
    (94, 15, &[3, 10]),
    (95, 5, &[32, 575]),
    (96, 5, &[32, 576]),
    (97, 5, &[32, 577]),
    (98, 5, &[84, 0, 0, 0, 0, 0]),
    (99, 5, &[32, 578]),
    (101, 5, &[32, 579]),
    (102, 5, &[32, 580]),
    (103, 18, &[1]),
    (104, 18, &[2]),
    (105, 5, &[113]),
    (106, 5, &[114]),
    (108, 15, &[3, 11]),
    (109, 15, &[3, 12]),
    (111, 5, &[32, 581]),
    (112, 5, &[32, 582]),
    (115, 18, &[4]),
    (116, 18, &[5]),
    (117, 18, &[6]),
    (118, 18, &[3]),
    (119, 5, &[32, 583]),
    (120, 5, &[32, 584]),
    (121, 5, &[32, 585]),
    (122, 5, &[32, 586]),
];
/// CreateWellKnownSid's SID for a type that needs no domain.
pub fn well_known(kind: u32) -> Option<Sid> {
    WELL_KNOWN
        .iter()
        .find(|(k, _, _)| *k == kind)
        .map(|(_, a, s)| Sid::new(*a, s))
}
/// IsWellKnownSid: whether `sid` is the type's SID.
pub fn is_well_known(sid: &Sid, kind: u32) -> bool {
    well_known(kind).is_some_and(|w| w == *sid)
}
/// Names LookupAccountSid gives the well-known SIDs: (authority,
/// sub-authorities, domain, name, SID_NAME_USE). SIDs Windows cannot name
/// (ERROR_NONE_MAPPED) are absent.
const NAMES: &[(u64, &[u32], &str, &str, u32)] = &[
    (0, &[0], "", "NULL SID", 5),
    (1, &[0], "", "Everyone", 5),
    (2, &[0], "", "LOCAL", 5),
    (2, &[1], "", "CONSOLE LOGON", 5),
    (3, &[0], "", "CREATOR OWNER", 5),
    (3, &[1], "", "CREATOR GROUP", 5),
    (3, &[2], "", "CREATOR OWNER SERVER", 5),
    (3, &[3], "", "CREATOR GROUP SERVER", 5),
    (3, &[4], "", "OWNER RIGHTS", 5),
    (5, &[], "NT Pseudo Domain", "NT Pseudo Domain", 3),
    (5, &[1], "NT AUTHORITY", "DIALUP", 5),
    (5, &[2], "NT AUTHORITY", "NETWORK", 5),
    (5, &[3], "NT AUTHORITY", "BATCH", 5),
    (5, &[4], "NT AUTHORITY", "INTERACTIVE", 5),
    (5, &[6], "NT AUTHORITY", "SERVICE", 5),
    (5, &[7], "NT AUTHORITY", "ANONYMOUS LOGON", 5),
    (5, &[8], "NT AUTHORITY", "PROXY", 5),
    (5, &[9], "NT AUTHORITY", "ENTERPRISE DOMAIN CONTROLLERS", 5),
    (5, &[10], "NT AUTHORITY", "SELF", 5),
    (5, &[11], "NT AUTHORITY", "Authenticated Users", 5),
    (5, &[12], "NT AUTHORITY", "RESTRICTED", 5),
    (5, &[13], "NT AUTHORITY", "TERMINAL SERVER USER", 5),
    (5, &[14], "NT AUTHORITY", "REMOTE INTERACTIVE LOGON", 5),
    (5, &[15], "NT AUTHORITY", "This Organization", 5),
    (5, &[17], "NT AUTHORITY", "IUSR", 5),
    (5, &[18], "NT AUTHORITY", "SYSTEM", 5),
    (5, &[19], "NT AUTHORITY", "LOCAL SERVICE", 5),
    (5, &[20], "NT AUTHORITY", "NETWORK SERVICE", 5),
    (
        5,
        &[22],
        "NT AUTHORITY",
        "ENTERPRISE READ-ONLY DOMAIN CONTROLLERS BETA",
        5,
    ),
    (5, &[33], "NT AUTHORITY", "WRITE RESTRICTED", 5),
    (5, &[1000], "NT AUTHORITY", "Other Organization", 5),
    (5, &[64, 10], "NT AUTHORITY", "NTLM Authentication", 5),
    (5, &[64, 14], "NT AUTHORITY", "SChannel Authentication", 5),
    (5, &[64, 21], "NT AUTHORITY", "Digest Authentication", 5),
    (
        5,
        &[65, 1],
        "NT AUTHORITY",
        "This Organization Certificate",
        5,
    ),
    (
        5,
        &[84, 0, 0, 0, 0, 0],
        "NT AUTHORITY",
        "USER MODE DRIVERS",
        5,
    ),
    (5, &[113], "NT AUTHORITY", "Local account", 5),
    (
        5,
        &[114],
        "NT AUTHORITY",
        "Local account and member of Administrators group",
        5,
    ),
    (5, &[32], "BUILTIN", "BUILTIN", 3),
    (5, &[32, 544], "BUILTIN", "Administrators", 4),
    (5, &[32, 545], "BUILTIN", "Users", 4),
    (5, &[32, 546], "BUILTIN", "Guests", 4),
    (5, &[32, 547], "BUILTIN", "Power Users", 4),
    (5, &[32, 551], "BUILTIN", "Backup Operators", 4),
    (5, &[32, 552], "BUILTIN", "Replicator", 4),
    (5, &[32, 555], "BUILTIN", "Remote Desktop Users", 4),
    (
        5,
        &[32, 556],
        "BUILTIN",
        "Network Configuration Operators",
        4,
    ),
    (5, &[32, 558], "BUILTIN", "Performance Monitor Users", 4),
    (5, &[32, 559], "BUILTIN", "Performance Log Users", 4),
    (5, &[32, 562], "BUILTIN", "Distributed COM Users", 4),
    (5, &[32, 568], "BUILTIN", "IIS_IUSRS", 4),
    (5, &[32, 569], "BUILTIN", "Cryptographic Operators", 4),
    (5, &[32, 573], "BUILTIN", "Event Log Readers", 4),
    (5, &[32, 578], "BUILTIN", "Hyper-V Administrators", 4),
    (
        5,
        &[32, 579],
        "BUILTIN",
        "Access Control Assistance Operators",
        4,
    ),
    (5, &[32, 580], "BUILTIN", "Remote Management Users", 4),
    (5, &[32, 581], "BUILTIN", "System Managed Accounts Group", 4),
    (5, &[32, 583], "BUILTIN", "Device Owners", 4),
    (5, &[32, 584], "BUILTIN", "User Mode Hardware Operators", 4),
    (5, &[32, 585], "BUILTIN", "OpenSSH Users", 4),
    (5, &[32, 586], "BUILTIN", "Agent Users", 4),
    (16, &[0], "Mandatory Label", "Untrusted Mandatory Level", 10),
    (16, &[4096], "Mandatory Label", "Low Mandatory Level", 10),
    (16, &[8192], "Mandatory Label", "Medium Mandatory Level", 10),
    (
        16,
        &[8448],
        "Mandatory Label",
        "Medium Plus Mandatory Level",
        10,
    ),
    (16, &[12288], "Mandatory Label", "High Mandatory Level", 10),
    (
        16,
        &[16384],
        "Mandatory Label",
        "System Mandatory Level",
        10,
    ),
    (
        15,
        &[2, 1],
        "APPLICATION PACKAGE AUTHORITY",
        "ALL APPLICATION PACKAGES",
        5,
    ),
    (
        15,
        &[3, 1],
        "APPLICATION PACKAGE AUTHORITY",
        "Your Internet connection",
        5,
    ),
    (
        15,
        &[3, 2],
        "APPLICATION PACKAGE AUTHORITY",
        "Your Internet connection, including incoming connections from the Internet",
        5,
    ),
    (
        15,
        &[3, 3],
        "APPLICATION PACKAGE AUTHORITY",
        "Your home or work networks",
        5,
    ),
    (
        15,
        &[3, 4],
        "APPLICATION PACKAGE AUTHORITY",
        "Your pictures library",
        5,
    ),
    (
        15,
        &[3, 5],
        "APPLICATION PACKAGE AUTHORITY",
        "Your videos library",
        5,
    ),
    (
        15,
        &[3, 6],
        "APPLICATION PACKAGE AUTHORITY",
        "Your music library",
        5,
    ),
    (
        15,
        &[3, 7],
        "APPLICATION PACKAGE AUTHORITY",
        "Your documents library",
        5,
    ),
    (
        15,
        &[3, 8],
        "APPLICATION PACKAGE AUTHORITY",
        "Your Windows credentials",
        5,
    ),
    (
        15,
        &[3, 9],
        "APPLICATION PACKAGE AUTHORITY",
        "Software and hardware certificates or a smart card",
        5,
    ),
    (
        15,
        &[3, 10],
        "APPLICATION PACKAGE AUTHORITY",
        "Removable storage",
        5,
    ),
    (
        15,
        &[3, 11],
        "APPLICATION PACKAGE AUTHORITY",
        "Your Appointments",
        5,
    ),
    (
        15,
        &[3, 12],
        "APPLICATION PACKAGE AUTHORITY",
        "Your Contacts",
        5,
    ),
    (
        18,
        &[1],
        "",
        "Authentication authority asserted identity",
        5,
    ),
    (18, &[2], "", "Service asserted identity", 5),
    (18, &[3], "", "Fresh public key identity", 5),
    (18, &[4], "", "Key trust identity", 5),
    (18, &[5], "", "Key property multi-factor authentication", 5),
    (18, &[6], "", "Key property attestation", 5),
];
/// The local accounts' domain marker: LookupAccountSid answers the
/// computer's name for it.
pub const LOCAL_DOMAIN: &str = "DOORS";
/// (domain, name, SID_NAME_USE) for `sid`: the well-known SIDs, logon
/// sessions, the "None" group and the accounts `user` names (uid, name).
pub fn account(sid: &Sid, user: (u32, &str)) -> Option<(String, String, u32)> {
    if let Some((_, _, d, n, u)) = NAMES
        .iter()
        .find(|(a, s, _, _, _)| *a == sid.authority && *s == sid.sub.as_slice())
    {
        return Some((String::from(*d), String::from(*n), *u));
    }
    match (sid.authority, sid.sub.as_slice()) {
        // A logon session: SidTypeLogonSession.
        (5, [5, high, low]) => Some((
            String::from("NT AUTHORITY"),
            format!("LogonSessionId_{high}_{low}"),
            11,
        )),
        (5, [21, a, b, c, rid]) if [*a, *b, *c] == MACHINE => match *rid {
            513 => Some((String::from(LOCAL_DOMAIN), String::from("None"), 2)),
            rid if rid == user.0 => Some((String::from(LOCAL_DOMAIN), String::from(user.1), 1)),
            _ => None,
        },
        _ => None,
    }
}
/// LookupAccountName: "name" or "domain\name" (the computer's name or
/// "DOORS" for local accounts). The empty name is the BUILTIN domain.
pub fn lookup_name(text: &str, computer: &str, user: (u32, &str)) -> Option<(Sid, String, u32)> {
    let (domain, name) = match text.split_once('\\') {
        Some((d, n)) => (Some(d), n),
        None => (None, text),
    };
    if domain.is_none() && name.is_empty() {
        return Some((Sid::new(5, &[32]), String::from("BUILTIN"), 3));
    }
    let local = |d: &str| d.eq_ignore_ascii_case(computer) || d.eq_ignore_ascii_case(LOCAL_DOMAIN);
    if domain.is_none_or(local) {
        if name.eq_ignore_ascii_case(user.1) {
            return Some((Sid::user(user.0), String::from(computer), 1));
        }
        if name.eq_ignore_ascii_case("None") {
            return Some((Sid::user(513), String::from(computer), 2));
        }
        if domain.is_some() {
            return None;
        }
    }
    NAMES
        .iter()
        .find(|(_, _, d, n, _)| {
            n.eq_ignore_ascii_case(name) && domain.is_none_or(|x| x.eq_ignore_ascii_case(d))
        })
        .map(|(a, s, d, _, u)| (Sid::new(*a, s), String::from(*d), *u))
}

// ---- SDDL SID aliases ------------------------------------------------------------------
const ALIASES: &[(&str, u64, &[u32])] = &[
    ("AA", 5, &[32, 579]),
    ("AC", 15, &[2, 1]),
    ("AN", 5, &[7]),
    ("AO", 5, &[32, 548]),
    ("AS", 18, &[1]),
    ("AU", 5, &[11]),
    ("BA", 5, &[32, 544]),
    ("BG", 5, &[32, 546]),
    ("BO", 5, &[32, 551]),
    ("BU", 5, &[32, 545]),
    ("CD", 5, &[32, 574]),
    ("CG", 3, &[1]),
    ("CO", 3, &[0]),
    ("CY", 5, &[32, 569]),
    ("ED", 5, &[9]),
    ("ER", 5, &[32, 573]),
    ("ES", 5, &[32, 576]),
    ("HA", 5, &[32, 578]),
    ("HI", 16, &[12288]),
    ("HO", 5, &[32, 584]),
    ("IS", 5, &[32, 568]),
    ("IU", 5, &[4]),
    ("LS", 5, &[19]),
    ("LU", 5, &[32, 559]),
    ("LW", 16, &[4096]),
    ("ME", 16, &[8192]),
    ("MP", 16, &[8448]),
    ("MS", 5, &[32, 577]),
    ("MU", 5, &[32, 558]),
    ("NO", 5, &[32, 556]),
    ("NS", 5, &[20]),
    ("NU", 5, &[2]),
    ("OW", 3, &[4]),
    ("PO", 5, &[32, 550]),
    ("PS", 5, &[10]),
    ("PU", 5, &[32, 547]),
    ("RA", 5, &[32, 575]),
    ("RC", 5, &[12]),
    ("RD", 5, &[32, 555]),
    ("RE", 5, &[32, 552]),
    ("RM", 5, &[32, 580]),
    ("RU", 5, &[32, 554]),
    ("SI", 16, &[16384]),
    ("SO", 5, &[32, 549]),
    ("SS", 18, &[2]),
    ("SU", 5, &[6]),
    ("SY", 5, &[18]),
    ("UD", 5, &[84, 0, 0, 0, 0, 0]),
    ("WD", 1, &[0]),
    ("WR", 5, &[33]),
];
/// The SID an SDDL alias names; LA and LG are the local Administrator and
/// Guest accounts.
pub fn alias_sid(alias: &str) -> Option<Sid> {
    match alias {
        "LA" => return Some(Sid::user(500)),
        "LG" => return Some(Sid::user(501)),
        _ => (),
    }
    ALIASES
        .iter()
        .find(|(a, _, _)| *a == alias)
        .map(|(_, auth, s)| Sid::new(*auth, s))
}
/// A SID in SDDL: its alias when it has one, else S-1-....
pub fn sid_sddl(sid: &Sid) -> String {
    if *sid == Sid::user(500) {
        return String::from("LA");
    }
    if *sid == Sid::user(501) {
        return String::from("LG");
    }
    ALIASES
        .iter()
        .find(|(_, a, s)| *a == sid.authority && *s == sid.sub.as_slice())
        .map_or_else(|| sid.text(), |(alias, _, _)| String::from(*alias))
}
/// ConvertStringSidToSid: "S-1-..." or a two-letter alias.
pub fn parse_sid(text: &str) -> Option<Sid> {
    if text.len() == 2 {
        return alias_sid(text);
    }
    Sid::parse(text)
}

// ---- ACLs and descriptors ----------------------------------------------------------------
/// ACE types.
pub mod ace {
    pub const ALLOWED: u8 = 0;
    pub const DENIED: u8 = 1;
    pub const AUDIT: u8 = 2;
    pub const ALARM: u8 = 3;
    pub const LABEL: u8 = 0x11;
    /// Flags.
    pub const OBJECT_INHERIT: u8 = 0x1;
    pub const CONTAINER_INHERIT: u8 = 0x2;
    pub const NO_PROPAGATE: u8 = 0x4;
    pub const INHERIT_ONLY: u8 = 0x8;
    pub const INHERITED: u8 = 0x10;
    pub const SUCCESSFUL: u8 = 0x40;
    pub const FAILED: u8 = 0x80;
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ace {
    pub kind: u8,
    pub flags: u8,
    pub mask: u32,
    pub sid: Sid,
}
impl Ace {
    pub fn new(kind: u8, flags: u8, mask: u32, sid: Sid) -> Self {
        Self {
            kind,
            flags,
            mask,
            sid,
        }
    }
    pub fn length(&self) -> usize {
        8 + self.sid.length()
    }
    pub fn encode(&self) -> Vec<u8> {
        let mut b = Vec::with_capacity(self.length());
        b.push(self.kind);
        b.push(self.flags);
        b.extend_from_slice(&(self.length() as u16).to_le_bytes());
        b.extend_from_slice(&self.mask.to_le_bytes());
        b.extend_from_slice(&self.sid.encode());
        b
    }
    /// An ACE at the start of `b`, and its stated size.
    pub fn decode(b: &[u8]) -> Option<(Self, usize)> {
        if b.len() < 8 {
            return None;
        }
        let size = usize::from(u16::from_le_bytes([b[2], b[3]]));
        if size < 8 || size > b.len() || !size.is_multiple_of(4) {
            return None;
        }
        let sid = Sid::decode(&b[8..size])?;
        if sid.length() > size - 8 {
            return None;
        }
        Some((
            Self {
                kind: b[0],
                flags: b[1],
                mask: u32::from_le_bytes(b[4..8].try_into().unwrap()),
                sid,
            },
            size,
        ))
    }
}
/// ACL_REVISION.
pub const ACL_REVISION: u8 = 2;
pub const ACL_REVISION_DS: u8 = 4;
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Acl {
    pub aces: Vec<Ace>,
}
impl Acl {
    pub fn length(&self) -> usize {
        8 + self.aces.iter().map(Ace::length).sum::<usize>()
    }
    pub fn encode(&self) -> Vec<u8> {
        let mut b = Vec::with_capacity(self.length());
        b.push(ACL_REVISION);
        b.push(0);
        b.extend_from_slice(&(self.length() as u16).to_le_bytes());
        b.extend_from_slice(&(self.aces.len() as u16).to_le_bytes());
        b.extend_from_slice(&[0, 0]);
        for a in &self.aces {
            b.extend_from_slice(&a.encode());
        }
        b
    }
    /// An ACL at the start of `b` (its header's AclSize bounds it).
    pub fn decode(b: &[u8]) -> Option<Self> {
        let (size, count) = header(b)?;
        let mut aces = Vec::with_capacity(count);
        let mut at = 8;
        for _ in 0..count {
            let (ace, n) = Ace::decode(&b[at..size])?;
            aces.push(ace);
            at += n;
        }
        Some(Self { aces })
    }
}
/// An ACL header's (AclSize, AceCount) when valid: revision 2..=4, size at
/// least the header and a multiple of four.
pub fn header(b: &[u8]) -> Option<(usize, usize)> {
    if b.len() < 8 || !(2..=4).contains(&b[0]) {
        return None;
    }
    let size = usize::from(u16::from_le_bytes([b[2], b[3]]));
    let count = usize::from(u16::from_le_bytes([b[4], b[5]]));
    (size >= 8 && size <= b.len() && size.is_multiple_of(4)).then_some((size, count))
}
/// SECURITY_DESCRIPTOR_CONTROL bits.
pub mod control {
    pub const OWNER_DEFAULTED: u16 = 0x1;
    pub const GROUP_DEFAULTED: u16 = 0x2;
    pub const DACL_PRESENT: u16 = 0x4;
    pub const DACL_DEFAULTED: u16 = 0x8;
    pub const SACL_PRESENT: u16 = 0x10;
    pub const SACL_DEFAULTED: u16 = 0x20;
    pub const DACL_AUTO_INHERIT_REQ: u16 = 0x100;
    pub const SACL_AUTO_INHERIT_REQ: u16 = 0x200;
    pub const DACL_AUTO_INHERITED: u16 = 0x400;
    pub const SACL_AUTO_INHERITED: u16 = 0x800;
    pub const DACL_PROTECTED: u16 = 0x1000;
    pub const SACL_PROTECTED: u16 = 0x2000;
    pub const RM_CONTROL_VALID: u16 = 0x4000;
    pub const SELF_RELATIVE: u16 = 0x8000;
    /// The bits SetSecurityDescriptorControl may change.
    pub const SETTABLE: u16 = DACL_AUTO_INHERIT_REQ
        | SACL_AUTO_INHERIT_REQ
        | DACL_AUTO_INHERITED
        | SACL_AUTO_INHERITED
        | DACL_PROTECTED
        | SACL_PROTECTED;
}
/// SECURITY_INFORMATION bits.
pub mod info {
    pub const OWNER: u32 = 0x1;
    pub const GROUP: u32 = 0x2;
    pub const DACL: u32 = 0x4;
    pub const SACL: u32 = 0x8;
    pub const LABEL: u32 = 0x10;
    pub const PROTECTED_DACL: u32 = 0x8000_0000;
    pub const PROTECTED_SACL: u32 = 0x4000_0000;
    pub const UNPROTECTED_DACL: u32 = 0x2000_0000;
    pub const UNPROTECTED_SACL: u32 = 0x1000_0000;
}
/// A security descriptor's parts. `dacl`/`sacl` are None for a NULL ACL;
/// presence is the control's DACL_PRESENT / SACL_PRESENT.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Descriptor {
    pub control: u16,
    pub owner: Option<Sid>,
    pub group: Option<Sid>,
    pub sacl: Option<Acl>,
    pub dacl: Option<Acl>,
}
/// SECURITY_DESCRIPTOR_MIN_LENGTH on x64 (the absolute form).
pub const ABSOLUTE_BYTES: usize = 40;
pub const RELATIVE_HEADER: usize = 20;
impl Descriptor {
    pub fn dacl_present(&self) -> bool {
        self.control & control::DACL_PRESENT != 0
    }
    pub fn sacl_present(&self) -> bool {
        self.control & control::SACL_PRESENT != 0
    }
    /// The self-relative form, as Windows lays it out: header, SACL, DACL,
    /// owner, group.
    pub fn relative(&self) -> Vec<u8> {
        let mut b = Vec::new();
        b.push(1);
        b.push(0);
        b.extend_from_slice(&(self.control | control::SELF_RELATIVE).to_le_bytes());
        b.resize(RELATIVE_HEADER, 0);
        let place = |b: &mut Vec<u8>, slot: usize, bytes: Option<Vec<u8>>| {
            if let Some(bytes) = bytes {
                let at = b.len() as u32;
                b[slot..slot + 4].copy_from_slice(&at.to_le_bytes());
                b.extend_from_slice(&bytes);
            }
        };
        let sacl = self
            .sacl
            .as_ref()
            .filter(|_| self.sacl_present())
            .map(Acl::encode);
        let dacl = self
            .dacl
            .as_ref()
            .filter(|_| self.dacl_present())
            .map(Acl::encode);
        place(&mut b, 12, sacl);
        place(&mut b, 16, dacl);
        place(&mut b, 4, self.owner.as_ref().map(Sid::encode));
        place(&mut b, 8, self.group.as_ref().map(Sid::encode));
        b
    }
    /// A self-relative descriptor.
    pub fn from_relative(b: &[u8]) -> Option<Self> {
        if b.len() < RELATIVE_HEADER || b[0] != 1 {
            return None;
        }
        let control = u16::from_le_bytes([b[2], b[3]]);
        if control & control::SELF_RELATIVE == 0 {
            return None;
        }
        let at = |slot: usize| u32::from_le_bytes(b[slot..slot + 4].try_into().unwrap()) as usize;
        let sid = |offset: usize| -> Option<Option<Sid>> {
            if offset == 0 {
                return Some(None);
            }
            b.get(offset..).and_then(Sid::decode).map(Some)
        };
        let acl = |offset: usize, present: bool| -> Option<Option<Acl>> {
            if offset == 0 || !present {
                return Some(None);
            }
            b.get(offset..).and_then(Acl::decode).map(Some)
        };
        Some(Self {
            control: control & !control::SELF_RELATIVE,
            owner: sid(at(4))?,
            group: sid(at(8))?,
            sacl: acl(at(12), control & control::SACL_PRESENT != 0)?,
            dacl: acl(at(16), control & control::DACL_PRESENT != 0)?,
        })
    }
    /// The bytes a self-relative descriptor at the start of `b` occupies.
    pub fn relative_length(b: &[u8]) -> Option<usize> {
        let d = Self::from_relative(b)?;
        let at = |slot: usize| u32::from_le_bytes(b[slot..slot + 4].try_into().unwrap()) as usize;
        let mut end = RELATIVE_HEADER;
        if let Some(o) = &d.owner {
            end = end.max(at(4) + o.length());
        }
        if let Some(g) = &d.group {
            end = end.max(at(8) + g.length());
        }
        if let Some(s) = &d.sacl {
            end = end.max(at(12) + s.length());
        }
        if let Some(a) = &d.dacl {
            end = end.max(at(16) + a.length());
        }
        Some(end)
    }
    /// The parts `which` (SECURITY_INFORMATION) selects; LABEL keeps only
    /// the SACL's label ACEs.
    pub fn select(&self, which: u32) -> Self {
        let mut d = Self::default();
        if which & info::OWNER != 0 {
            d.owner.clone_from(&self.owner);
        }
        if which & info::GROUP != 0 {
            d.group.clone_from(&self.group);
        }
        if which & info::DACL != 0 {
            d.control |= self.control
                & (control::DACL_PRESENT
                    | control::DACL_DEFAULTED
                    | control::DACL_PROTECTED
                    | control::DACL_AUTO_INHERITED
                    | control::DACL_AUTO_INHERIT_REQ);
            d.dacl.clone_from(&self.dacl);
        }
        if which & (info::SACL | info::LABEL) != 0 && self.sacl_present() {
            d.control |= self.control
                & (control::SACL_PRESENT
                    | control::SACL_DEFAULTED
                    | control::SACL_PROTECTED
                    | control::SACL_AUTO_INHERITED
                    | control::SACL_AUTO_INHERIT_REQ);
            d.sacl = self.sacl.as_ref().map(|s| Acl {
                aces: s
                    .aces
                    .iter()
                    .filter(|a| which & info::SACL != 0 || a.kind == ace::LABEL)
                    .cloned()
                    .collect(),
            });
        }
        d
    }
}

// ---- SDDL -------------------------------------------------------------------------------
const RIGHT_LETTERS: [(u32, &str); 17] = [
    (0x1, "CC"),
    (0x2, "DC"),
    (0x4, "LC"),
    (0x8, "SW"),
    (0x10, "RP"),
    (0x20, "WP"),
    (0x40, "DT"),
    (0x80, "LO"),
    (0x100, "CR"),
    (0x1_0000, "SD"),
    (0x2_0000, "RC"),
    (0x4_0000, "WD"),
    (0x8_0000, "WO"),
    (0x1000_0000, "GA"),
    (0x2000_0000, "GX"),
    (0x4000_0000, "GW"),
    (0x8000_0000, "GR"),
];
/// Whole-mask aliases (KX is KR's mask; Windows writes KR).
const RIGHT_ALIASES: [(u32, &str); 8] = [
    (0x1F_01FF, "FA"),
    (0x12_0089, "FR"),
    (0x12_0116, "FW"),
    (0x12_00A0, "FX"),
    (0xF_003F, "KA"),
    (0x2_0019, "KR"),
    (0x2_0006, "KW"),
    (0x2_0019, "KX"),
];
const LABEL_LETTERS: [(u32, &str); 3] = [(0x1, "NW"), (0x2, "NR"), (0x4, "NX")];
/// An ACE's rights as Windows writes them: a whole-mask alias, else letters
/// by ascending bit, else hexadecimal when a bit has no letter.
pub fn rights_text(mask: u32, label: bool) -> String {
    if !label && let Some((_, a)) = RIGHT_ALIASES.iter().find(|(m, _)| *m == mask) {
        return String::from(*a);
    }
    let letters: &[(u32, &str)] = if label {
        &LABEL_LETTERS
    } else {
        &RIGHT_LETTERS
    };
    let known = letters.iter().fold(0, |m, (b, _)| m | b);
    if mask == 0 || mask & !known != 0 {
        return format!("0x{mask:x}");
    }
    letters
        .iter()
        .filter(|(b, _)| mask & b != 0)
        .map(|(_, l)| *l)
        .collect()
}
fn parse_rights(text: &str) -> Option<u32> {
    if let Some(hex) = text.strip_prefix("0x").or_else(|| text.strip_prefix("0X")) {
        return u32::from_str_radix(hex, 16).ok();
    }
    if !text.is_empty() && text.bytes().all(|b| b.is_ascii_digit()) {
        return text.parse().ok();
    }
    if !text.len().is_multiple_of(2) {
        return None;
    }
    let mut mask = 0;
    for i in (0..text.len()).step_by(2) {
        let t = text.get(i..i + 2)?;
        mask |= RIGHT_ALIASES
            .iter()
            .chain(RIGHT_LETTERS.iter())
            .chain(LABEL_LETTERS.iter())
            .find(|(_, l)| *l == t)?
            .0;
    }
    Some(mask)
}
const FLAG_LETTERS: [(u8, &str); 7] = [
    (ace::OBJECT_INHERIT, "OI"),
    (ace::CONTAINER_INHERIT, "CI"),
    (ace::NO_PROPAGATE, "NP"),
    (ace::INHERIT_ONLY, "IO"),
    (ace::INHERITED, "ID"),
    (ace::SUCCESSFUL, "SA"),
    (ace::FAILED, "FA"),
];
const TYPE_LETTERS: [(u8, &str); 5] = [
    (ace::ALLOWED, "A"),
    (ace::DENIED, "D"),
    (ace::AUDIT, "AU"),
    (ace::ALARM, "AL"),
    (ace::LABEL, "ML"),
];
fn ace_text(a: &Ace) -> Option<String> {
    let kind = TYPE_LETTERS.iter().find(|(k, _)| *k == a.kind)?.1;
    let flags: String = FLAG_LETTERS
        .iter()
        .filter(|(f, _)| a.flags & f != 0)
        .map(|(_, l)| *l)
        .collect();
    Some(format!(
        "({kind};{flags};{};;;{})",
        rights_text(a.mask, a.kind == ace::LABEL),
        sid_sddl(&a.sid)
    ))
}
fn acl_text(present: bool, acl: Option<&Acl>, control: u16, sacl: bool) -> Option<String> {
    if !present {
        return Some(String::new());
    }
    let (protected, required, inherited) = if sacl {
        (
            control::SACL_PROTECTED,
            control::SACL_AUTO_INHERIT_REQ,
            control::SACL_AUTO_INHERITED,
        )
    } else {
        (
            control::DACL_PROTECTED,
            control::DACL_AUTO_INHERIT_REQ,
            control::DACL_AUTO_INHERITED,
        )
    };
    let mut s = String::from(if sacl { "S:" } else { "D:" });
    for (bit, text) in [(protected, "P"), (required, "AR"), (inherited, "AI")] {
        if control & bit != 0 {
            s.push_str(text);
        }
    }
    match acl {
        None => s.push_str("NO_ACCESS_CONTROL"),
        Some(acl) => {
            for a in &acl.aces {
                s.push_str(&ace_text(a)?);
            }
        }
    }
    Some(s)
}
/// ConvertSecurityDescriptorToStringSecurityDescriptor: the parts `which`
/// selects, in Windows' order (O:, G:, D:, S:). None for an ACE type SDDL
/// cannot write.
pub fn to_sddl(d: &Descriptor, which: u32) -> Option<String> {
    let mut s = String::new();
    if which & info::OWNER != 0
        && let Some(o) = &d.owner
    {
        s.push_str("O:");
        s.push_str(&sid_sddl(o));
    }
    if which & info::GROUP != 0
        && let Some(g) = &d.group
    {
        s.push_str("G:");
        s.push_str(&sid_sddl(g));
    }
    if which & info::DACL != 0 {
        s.push_str(&acl_text(
            d.dacl_present(),
            d.dacl.as_ref(),
            d.control,
            false,
        )?);
    }
    if which & (info::SACL | info::LABEL) != 0 {
        let selected = d.select(which);
        s.push_str(&acl_text(
            selected.sacl_present(),
            selected.sacl.as_ref(),
            d.control,
            true,
        )?);
    }
    Some(s)
}
/// The length ConvertSecurityDescriptorToStringSecurityDescriptor reports
/// for `text` (measured on Windows 11, which sizes its buffer generously):
/// the text's, plus one per ACE and per NULL ACL, plus a hexadecimal
/// mask's padding to ten characters.
pub fn sddl_length(d: &Descriptor, which: u32, text: &str) -> usize {
    let mut n = text.encode_utf16().count();
    let mut count = |present: bool, acl: Option<&Acl>| {
        if !present {
            return;
        }
        let Some(acl) = acl else {
            n += 1;
            return;
        };
        for a in &acl.aces {
            n += 1;
            let rights = rights_text(a.mask, a.kind == ace::LABEL);
            if rights.starts_with("0x") {
                n += 10usize.saturating_sub(rights.len());
            }
        }
    };
    if which & info::DACL != 0 {
        count(d.dacl_present(), d.dacl.as_ref());
    }
    if which & (info::SACL | info::LABEL) != 0 {
        let selected = d.select(which);
        count(selected.sacl_present(), selected.sacl.as_ref());
    }
    n
}
/// SID characters after "O:"/"G:"/in an ACE: an alias (two letters) or
/// "S-" and digits/dashes.
fn take_sid(text: &str) -> Option<(Sid, &str)> {
    if text.starts_with("S-") || text.starts_with("s-") {
        let end = text[2..]
            .find(|c: char| !(c.is_ascii_digit() || c == '-'))
            .map_or(text.len(), |e| e + 2);
        return Some((Sid::parse(&text[..end])?, &text[end..]));
    }
    let alias = text.get(..2)?;
    Some((alias_sid(alias)?, &text[2..]))
}
fn parse_acl(mut text: &str, sacl: bool) -> Result<(u16, Option<Acl>, &str), u32> {
    let (protected, required, inherited) = if sacl {
        (
            control::SACL_PROTECTED,
            control::SACL_AUTO_INHERIT_REQ,
            control::SACL_AUTO_INHERITED,
        )
    } else {
        (
            control::DACL_PROTECTED,
            control::DACL_AUTO_INHERIT_REQ,
            control::DACL_AUTO_INHERITED,
        )
    };
    let mut bits = 0;
    loop {
        if let Some(rest) = text.strip_prefix("AR") {
            bits |= required;
            text = rest;
        } else if let Some(rest) = text.strip_prefix("AI") {
            bits |= inherited;
            text = rest;
        } else if let Some(rest) = text.strip_prefix('P') {
            bits |= protected;
            text = rest;
        } else {
            break;
        }
    }
    if let Some(rest) = text.strip_prefix("NO_ACCESS_CONTROL") {
        return Ok((bits, None, rest));
    }
    let mut acl = Acl::default();
    while let Some(rest) = text.strip_prefix('(') {
        let end = rest.find(')').ok_or(code::INVALID_ACL)?;
        let fields: Vec<&str> = rest[..end].split(';').collect();
        if fields.len() != 6 {
            return Err(code::INVALID_ACL);
        }
        let kind = TYPE_LETTERS
            .iter()
            .find(|(_, l)| *l == fields[0])
            .ok_or(code::INVALID_DATATYPE)?
            .0;
        let mut flags = 0u8;
        let mut f = fields[1];
        while !f.is_empty() {
            let t = f.get(..2).ok_or(code::INVALID_FLAGS)?;
            flags |= FLAG_LETTERS
                .iter()
                .find(|(_, l)| *l == t)
                .ok_or(code::INVALID_FLAGS)?
                .0;
            f = &f[2..];
        }
        let mask = parse_rights(fields[2]).ok_or(code::INVALID_ACL)?;
        if !fields[3].is_empty() || !fields[4].is_empty() {
            return Err(error::INVALID_PARAMETER);
        }
        let (sid, tail) = take_sid(fields[5]).ok_or(error::INVALID_SID)?;
        if !tail.is_empty() {
            return Err(error::INVALID_SID);
        }
        acl.aces.push(Ace::new(kind, flags, mask, sid));
        text = &rest[end + 1..];
    }
    Ok((bits, Some(acl), text))
}
/// ConvertStringSecurityDescriptorToSecurityDescriptor.
pub fn from_sddl(mut text: &str) -> Result<Descriptor, u32> {
    let mut d = Descriptor::default();
    while !text.is_empty() {
        let (part, rest) = text.split_at(text.find(':').ok_or(error::INVALID_PARAMETER)?);
        let rest = &rest[1..];
        match part {
            "O" | "G" => {
                let (sid, tail) = take_sid(rest).ok_or(error::INVALID_SID)?;
                if part == "O" {
                    d.owner = Some(sid);
                } else {
                    d.group = Some(sid);
                }
                text = tail;
            }
            "D" | "S" => {
                let sacl = part == "S";
                let (bits, acl, tail) = parse_acl(rest, sacl)?;
                d.control |= bits
                    | if sacl {
                        control::SACL_PRESENT
                    } else {
                        control::DACL_PRESENT
                    };
                if sacl {
                    d.sacl = acl;
                } else {
                    d.dacl = acl;
                }
                text = tail;
            }
            _ => return Err(error::INVALID_PARAMETER),
        }
    }
    Ok(d)
}

// ---- access checks -------------------------------------------------------------------
/// GENERIC_MAPPING: read, write, execute, all.
pub type Mapping = [u32; 4];
pub const FILE_MAPPING: Mapping = [0x12_0089, 0x12_0116, 0x12_00A0, 0x1F_01FF];
pub const KEY_MAPPING: Mapping = [0x2_0019, 0x2_0006, 0x2_0019, 0xF_003F];
pub const MAXIMUM_ALLOWED: u32 = 0x0200_0000;
const READ_CONTROL: u32 = 0x2_0000;
const WRITE_DAC: u32 = 0x4_0000;
/// MapGenericMask.
pub fn map_generic(mask: u32, m: &Mapping) -> u32 {
    let mut out = mask & 0x0FFF_FFFF;
    for (bit, mapped) in [0x8000_0000, 0x4000_0000, 0x2000_0000, 0x1000_0000]
        .iter()
        .zip(m)
    {
        if mask & bit != 0 {
            out |= mapped;
        }
    }
    out
}
/// What the token holds for an access check: its user and groups with their
/// attributes (SE_GROUP_ENABLED, SE_GROUP_USE_FOR_DENY_ONLY).
pub struct Holder<'a> {
    pub user: &'a Sid,
    pub groups: &'a [(Sid, u32)],
}
impl Holder<'_> {
    fn allows(&self, sid: &Sid) -> bool {
        sid == self.user
            || self
                .groups
                .iter()
                .any(|(g, a)| g == sid && a & 0x4 != 0 && a & 0x10 == 0)
    }
    fn denies(&self, sid: &Sid) -> bool {
        sid == self.user || self.groups.iter().any(|(g, a)| g == sid && a & 0x14 != 0)
    }
}
/// AccessCheck: Ok(granted) or Err(ERROR_ACCESS_DENIED) for a complete
/// descriptor; Err(ERROR_INVALID_SECURITY_DESCR) without owner or group.
pub fn access_check(
    d: &Descriptor,
    holder: &Holder,
    desired: u32,
    m: &Mapping,
) -> Result<u32, u32> {
    let (Some(owner), Some(_)) = (&d.owner, &d.group) else {
        return Err(code::INVALID_SECURITY_DESCR);
    };
    let maximum = desired & MAXIMUM_ALLOWED != 0;
    let wanted = map_generic(desired & !MAXIMUM_ALLOWED, m);
    let Some(dacl) = d.dacl.as_ref().filter(|_| d.dacl_present()) else {
        // No DACL, or a NULL one: everything.
        return Ok(if maximum { m[3] | wanted } else { wanted });
    };
    let owner_rights = Sid::new(3, &[4]);
    let mut granted = 0u32;
    let mut denied = 0u32;
    if holder.allows(owner) && !dacl.aces.iter().any(|a| a.sid == owner_rights) {
        granted |= READ_CONTROL | WRITE_DAC;
    }
    for a in &dacl.aces {
        if a.flags & ace::INHERIT_ONLY != 0 {
            continue;
        }
        let sid = if a.sid == Sid::new(3, &[4]) {
            owner
        } else {
            &a.sid
        };
        match a.kind {
            ace::ALLOWED if holder.allows(sid) => granted |= a.mask & !denied,
            ace::DENIED if holder.denies(sid) => denied |= a.mask & !granted,
            _ => continue,
        }
        if !maximum && granted & wanted == wanted {
            break;
        }
    }
    if maximum {
        return if granted & !denied == 0 && wanted == 0 {
            Err(error::ACCESS_DENIED)
        } else if granted & wanted == wanted {
            Ok(granted & !denied)
        } else {
            Err(error::ACCESS_DENIED)
        };
    }
    if granted & wanted == wanted {
        Ok(wanted)
    } else {
        Err(error::ACCESS_DENIED)
    }
}

// ---- Doors' storage ACLs as Windows descriptors ------------------------------------------------
use door_service_protocol::files::{Security, SecurityEntry, access, principal};
/// The SID of a storage owner or user entry: the system and services are
/// SYSTEM, accounts their local SIDs.
pub fn uid_sid(uid: u32) -> Sid {
    if uid == door_service_protocol::files::ADMINISTRATORS_OWNER {
        return Sid::administrators();
    }
    if uid < 1000 {
        Sid::local_system()
    } else {
        Sid::user(uid)
    }
}
fn group_sid(bit: u32) -> Option<Sid> {
    Some(match bit {
        1 => Sid::users(),
        2 => Sid::new(5, &[6]),
        3 => Sid::administrators(),
        4 => Sid::new(5, &[114]),
        _ => return None,
    })
}
/// A storage access mask as file rights: read is FILE_GENERIC_READ, traverse
/// FILE_EXECUTE, write and create FILE_GENERIC_WRITE (create alone
/// FILE_ADD_FILE | FILE_ADD_SUBDIRECTORY), delete DELETE, change
/// WRITE_DAC | WRITE_OWNER; all is FILE_ALL_ACCESS.
pub fn file_rights(bits: u16) -> u32 {
    if bits == access::ALL {
        return 0x1F_01FF;
    }
    let mut m = 0;
    if bits & access::READ != 0 {
        m |= 0x12_0089;
    }
    if bits & access::TRAVERSE != 0 {
        m |= 0x12_00A0;
    }
    if bits & access::WRITE != 0 {
        m |= 0x12_0116;
    }
    if bits & access::CREATE != 0 {
        m |= 0x6;
    }
    if bits & access::DELETE != 0 {
        m |= 0x1_0000;
    }
    if bits & access::CHANGE_ACL != 0 {
        m |= 0xC_0000;
    }
    m
}
/// File rights (generic bits mapped) as storage access: writing data is
/// also creating entries (the same bits on a folder).
pub fn storage_bits(rights: u32) -> u16 {
    let m = map_generic(rights, &FILE_MAPPING);
    let mut bits = 0;
    if m & 0x1 != 0 {
        bits |= access::READ;
    }
    if m & 0x20 != 0 {
        bits |= access::TRAVERSE;
    }
    if m & 0x6 != 0 {
        bits |= access::WRITE | access::CREATE;
    }
    if m & 0x1_0040 != 0 {
        bits |= access::DELETE;
    }
    if m & 0xC_0000 != 0 {
        bits |= access::CHANGE_ACL;
    }
    bits
}
/// A storage ACL as the descriptor GetNamedSecurityInfo returns: the owner
/// (its group is the owner too, as the reference profile shows), the
/// entries in order (inheriting ones OI|CI on a folder), and the mandatory
/// label (1 Medium, 2 System; no-write-up).
pub fn descriptor_of(s: &Security) -> Descriptor {
    let owner = uid_sid(s.owner);
    let inherit = if s.directory {
        ace::OBJECT_INHERIT | ace::CONTAINER_INHERIT
    } else {
        0
    };
    let mut dacl = Acl::default();
    for e in s.entries() {
        let sid = match e.principal {
            principal::EVERYONE => Sid::everyone(),
            principal::USER => uid_sid(e.id),
            _ => match group_sid(e.id) {
                Some(sid) => sid,
                None => continue,
            },
        };
        dacl.aces.push(Ace::new(
            if e.deny { ace::DENIED } else { ace::ALLOWED },
            if e.inherit { inherit } else { 0 },
            file_rights(e.access),
            sid,
        ));
    }
    let mut d = Descriptor {
        control: control::DACL_PRESENT,
        group: Some(owner.clone()),
        owner: Some(owner),
        dacl: Some(dacl),
        sacl: None,
    };
    if s.integrity != 0 {
        d.control |= control::SACL_PRESENT;
        d.sacl = Some(Acl {
            aces: alloc::vec![Ace::new(
                ace::LABEL,
                inherit,
                0x1,
                Sid::integrity(if s.integrity == 2 { 0x4000 } else { 0x2000 })
            )],
        });
    }
    d
}
/// The storage principal a SID names: every session application holds
/// Users and the logon groups (Authenticated Users, INTERACTIVE, Local
/// account, CONSOLE LOGON, LOCAL, This Organization), so those are Users.
/// None for a SID no Doors token holds (an entry for it can never apply).
fn principal_of(sid: &Sid) -> Option<(u8, u32)> {
    Some(match (sid.authority, sid.sub.as_slice()) {
        (1, [0]) => (principal::EVERYONE, 0),
        (5, [18]) => (principal::USER, 1),
        (5, [32, 545] | [11] | [4] | [113] | [15]) | (2, [0] | [1]) => (principal::GROUP, 1),
        (5, [6]) => (principal::GROUP, 2),
        (5, [32, 544]) => (principal::GROUP, 3),
        (5, [114]) => (principal::GROUP, 4),
        (5, [21, a, b, c, rid]) if [*a, *b, *c] == MACHINE && *rid >= 1000 => {
            (principal::USER, *rid)
        }
        _ => return None,
    })
}
/// SetNamedSecurityInfo on a file: `current` with the parts `which` selects
/// replaced from `d`. Entries a Doors token can never match and
/// inherit-only entries (Doors copies inheritance at creation) are dropped;
/// more than four remaining entries are ERROR_NOT_SUPPORTED.
pub fn apply(current: &Security, which: u32, d: &Descriptor) -> Result<Security, u32> {
    let mut out = *current;
    if which & info::OWNER != 0 {
        let owner = d.owner.as_ref().ok_or(code::INVALID_OWNER)?;
        out.owner = match principal_of(owner) {
            Some((principal::USER, 1)) if current.owner < 1000 => current.owner,
            Some((principal::USER, uid)) => uid,
            // Administrators (storage decides whether the caller may).
            Some((principal::GROUP, 3)) => door_service_protocol::files::ADMINISTRATORS_OWNER,
            _ => return Err(code::INVALID_OWNER),
        };
    }
    if which & info::DACL != 0 {
        let mut entries = [SecurityEntry::default(); 4];
        let mut count = 0usize;
        match d.dacl.as_ref().filter(|_| d.dacl_present()) {
            // A NULL DACL: everyone, everything.
            None => {
                entries[0] = SecurityEntry {
                    principal: principal::EVERYONE,
                    id: 0,
                    deny: false,
                    access: access::ALL,
                    inherit: true,
                };
                count = 1;
            }
            Some(acl) => {
                for a in &acl.aces {
                    if a.flags & ace::INHERIT_ONLY != 0
                        || !matches!(a.kind, ace::ALLOWED | ace::DENIED)
                    {
                        continue;
                    }
                    let Some((kind, id)) = principal_of(&a.sid) else {
                        continue;
                    };
                    let bits = storage_bits(a.mask);
                    let inherit = a.flags & (ace::OBJECT_INHERIT | ace::CONTAINER_INHERIT) != 0
                        || !current.directory;
                    let entry = SecurityEntry {
                        principal: kind,
                        id,
                        deny: a.kind == ace::DENIED,
                        access: bits,
                        inherit,
                    };
                    // Merge an entry for the same principal and kind.
                    if let Some(e) = entries[..count]
                        .iter_mut()
                        .find(|e| e.principal == kind && e.id == id && e.deny == entry.deny)
                    {
                        e.access |= bits;
                        e.inherit |= inherit;
                        continue;
                    }
                    if count == 4 {
                        return Err(error::NOT_SUPPORTED);
                    }
                    entries[count] = entry;
                    count += 1;
                }
            }
        }
        out.entries = entries;
        out.count = count as u8;
    }
    if which & info::LABEL != 0 {
        out.integrity = match d
            .sacl
            .as_ref()
            .and_then(|s| s.aces.iter().find(|a| a.kind == ace::LABEL))
        {
            None => 0,
            Some(a) => match a.sid.sub.as_slice() {
                [level] if *level >= 0x4000 => 2,
                [level] if *level >= 0x2000 => 1,
                _ => 0,
            },
        };
    }
    Ok(out)
}
/// The registry's descriptors as Doors enforces them: HKEY_LOCAL_MACHINE
/// (and HKEY_CLASSES_ROOT) read by Users and changed by Administrators and
/// the system, the user's own hive by that user and the system.
pub fn key_descriptor(machine: bool, uid: u32) -> Descriptor {
    let ci = ace::CONTAINER_INHERIT;
    let (owner, group, aces, control) = if machine {
        (
            Sid::administrators(),
            Sid::local_system(),
            alloc::vec![
                Ace::new(ace::ALLOWED, ci, 0x2_0019, Sid::users()),
                Ace::new(ace::ALLOWED, ci, 0xF_003F, Sid::administrators()),
                Ace::new(ace::ALLOWED, ci, 0xF_003F, Sid::local_system()),
            ],
            control::DACL_PROTECTED | control::DACL_AUTO_INHERITED,
        )
    } else {
        let oici = ace::OBJECT_INHERIT | ci;
        (
            Sid::local_system(),
            Sid::local_system(),
            alloc::vec![
                Ace::new(ace::ALLOWED, oici, 0xF_003F, Sid::user(uid)),
                Ace::new(ace::ALLOWED, oici, 0xF_003F, Sid::local_system()),
            ],
            control::DACL_PROTECTED | control::SACL_AUTO_INHERITED,
        )
    };
    Descriptor {
        // A label request finds no label: a present, NULL SACL.
        control: control | control::DACL_PRESENT | control::SACL_PRESENT,
        owner: Some(owner),
        group: Some(group),
        dacl: Some(Acl { aces }),
        sacl: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn hex(b: &[u8]) -> String {
        b.iter().map(|v| format!("{v:02x}")).collect()
    }
    #[test]
    fn sddl_round_trips_with_windows_bytes() {
        // ConvertStringSecurityDescriptorToSecurityDescriptor on Windows 11.
        let lengths = [65, 61, 15, 4, 20, 2, 50, 28];
        for (i, (sddl, bytes)) in [
            (
                "O:BAG:SYD:P(A;OICI;FA;;;SY)(A;;0x1200a9;;;BU)S:(ML;;NW;;;ME)",
                "010014906400000074000000140000003000000002001c00010000001100140001000000010100000000001000200000020034000200000000031400ff011f0001010000000000051200000000001800a90012000102000000000005200000002102000001020000000000052000000020020000010100000000000512000000",
            ),
            (
                "O:SYG:SYD:AI(A;ID;FA;;;SY)(A;OICIIOID;GA;;;BA)(D;;WD;;;WD)",
                "010004845c000000680000000000000014000000020048000300000000101400ff011f00010100000000000512000000001b180000000010010200000000000520000000200200000100140000000400010100000000000100000000010100000000000512000000010100000000000512000000",
            ),
            (
                "D:(A;;GA;;;WD)",
                "010004800000000000000000000000001400000002001c00010000000000140000000010010100000000000100000000",
            ),
            (
                "O:BA",
                "010000801400000000000000000000000000000001020000000000052000000020020000",
            ),
            (
                "D:NO_ACCESS_CONTROL",
                "0100048000000000000000000000000000000000",
            ),
            ("D:", "01000480000000000000000000000000140000000200080000000000"),
            (
                "G:S-1-5-21-1-2-3-513D:(A;;KR;;;AU)(A;CI;KA;;;CO)",
                "01000480000000004400000000000000140000000200300002000000000014001900020001010000000000050b000000000214003f000f0001010000000000030000000001050000000000051500000001000000020000000300000001020000",
            ),
            (
                "S:(ML;OICINPIO;NWNRNX;;;HI)",
                "010010800000000000000000140000000000000002001c0001000000110f140007000000010100000000001000300000",
            ),
        ]
        .into_iter()
        .enumerate()
        {
            let d = from_sddl(sddl).unwrap();
            assert_eq!(sddl_length(&d, 0x17, sddl), lengths[i], "{sddl}");
            assert_eq!(hex(&d.relative()), bytes, "{sddl}");
            let back = Descriptor::from_relative(&d.relative()).unwrap();
            assert_eq!(back, d);
            assert_eq!(Descriptor::relative_length(&d.relative()), Some(bytes.len() / 2));
            assert_eq!(to_sddl(&back, 0x1f).unwrap(), sddl, "{sddl}");
        }
    }
    #[test]
    fn sddl_errors_are_windows_codes() {
        // ConvertStringSecurityDescriptorToSecurityDescriptor's last errors.
        for (text, e) in [
            ("O:XX", 1337),
            ("D:(Q;;GA;;;WD)", 1804),
            ("D:(A;;GA;;;WD", 1336),
            ("garbage", 87),
            ("D:(A;;ZZ;;;WD)", 1336),
            ("D:(A;XX;GA;;;WD)", 1004),
            ("O:S-1-5-x", 1337),
        ] {
            assert_eq!(from_sddl(text).err(), Some(e), "{text}");
        }
        assert!(from_sddl("D:(A;;GA;;;S-1-5-32-544)").is_ok());
        assert_eq!(from_sddl(""), Ok(Descriptor::default()));
    }
    #[test]
    fn rights_are_written_as_windows_writes_them() {
        for (mask, text) in [
            (0x1000_0000, "GA"),
            (0xA000_0000, "GXGR"),
            (0xF000_0000, "GAGXGWGR"),
            (0x1F_01FF, "FA"),
            (0x12_0089, "FR"),
            (0x12_00A9, "0x1200a9"),
            (0x2_0019, "KR"),
            (0x2_001F, "CCDCLCSWRPRC"),
            (0xD_0000, "SDWDWO"),
            (0x1FF, "CCDCLCSWRPWPDTLOCR"),
            (0x10_0000, "0x100000"),
            (0x1F_0000, "0x1f0000"),
        ] {
            assert_eq!(rights_text(mask, false), text);
            assert_eq!(parse_rights(text), Some(mask));
        }
        assert_eq!(rights_text(7, true), "NWNRNX");
        assert_eq!(parse_rights("KX"), Some(0x2_0019));
        assert_eq!(parse_rights("ZZ"), None);
    }
    #[test]
    fn well_known_sids_aliases_and_names() {
        assert_eq!(well_known(26), Some(Sid::administrators()));
        assert_eq!(well_known(7), Some(Sid::new(5, &[])));
        assert_eq!(well_known(21), None);
        assert!(is_well_known(&Sid::local_system(), 22));
        assert_eq!(parse_sid("BA"), Some(Sid::administrators()));
        assert_eq!(parse_sid("AP"), None);
        assert_eq!(sid_sddl(&Sid::new(5, &[3])), "S-1-5-3");
        assert_eq!(sid_sddl(&Sid::local_system()), "SY");
        let user = (1001, "Ada");
        assert_eq!(
            account(&Sid::new(5, &[32]), user),
            Some((String::from("BUILTIN"), String::from("BUILTIN"), 3))
        );
        assert_eq!(account(&Sid::new(5, &[32, 548]), user), None);
        assert_eq!(account(&Sid::user(1001), user).unwrap().1, "Ada");
        assert_eq!(account(&Sid::user(513), user).unwrap().2, 2);
        assert_eq!(
            account(&Sid::new(5, &[5, 0, 7]), user).unwrap().1,
            "LogonSessionId_0_7"
        );
        let found = |n: &str| lookup_name(n, "PC", user).map(|(s, d, u)| (s.text(), d, u));
        assert_eq!(
            found("Administrators"),
            Some((String::from("S-1-5-32-544"), String::from("BUILTIN"), 4))
        );
        assert_eq!(found("NT AUTHORITY\\SYSTEM").unwrap().0, "S-1-5-18");
        assert_eq!(found("everyone").unwrap().1, "");
        assert_eq!(found("PC\\ada").unwrap().2, 1);
        assert_eq!(found("None").unwrap().2, 2);
        assert_eq!(found("").unwrap().0, "S-1-5-32");
        assert_eq!(found("NoSuchAccountDoors"), None);
    }
    #[test]
    fn access_checks_follow_ace_order_owner_rights_and_deny_only_groups() {
        let user = Sid::user(1001);
        let groups = [
            (Sid::everyone(), 7),
            (Sid::administrators(), 0x10),
            (Sid::users(), 7),
        ];
        let holder = Holder {
            user: &user,
            groups: &groups,
        };
        let d = from_sddl(&format!(
            "O:{}G:SYD:(D;;0x2;;;BA)(A;;FR;;;BU)(A;;FA;;;BA)",
            user.text()
        ))
        .unwrap();
        // Read through Users; write only through deny-only Administrators: no.
        assert_eq!(
            access_check(&d, &holder, 0x8000_0000, &FILE_MAPPING),
            Ok(0x12_0089)
        );
        assert_eq!(
            access_check(&d, &holder, 0x2, &FILE_MAPPING),
            Err(error::ACCESS_DENIED)
        );
        // The owner may read the descriptor and change it.
        assert_eq!(
            access_check(&d, &holder, READ_CONTROL | WRITE_DAC, &FILE_MAPPING),
            Ok(READ_CONTROL | WRITE_DAC)
        );
        assert_eq!(
            access_check(&d, &holder, MAXIMUM_ALLOWED, &FILE_MAPPING),
            Ok(0x12_0089 | WRITE_DAC)
        );
        let null = from_sddl("O:SYG:SYD:NO_ACCESS_CONTROL").unwrap();
        assert_eq!(access_check(&null, &holder, 0x2, &FILE_MAPPING), Ok(0x2));
        assert_eq!(
            access_check(&from_sddl("D:").unwrap(), &holder, 1, &FILE_MAPPING),
            Err(code::INVALID_SECURITY_DESCR)
        );
        assert_eq!(map_generic(0x8000_0001, &KEY_MAPPING), 0x2_0019);
    }
    #[test]
    fn storage_acls_become_descriptors_and_back() {
        let mut s = Security {
            owner: 1,
            integrity: 1,
            directory: true,
            count: 2,
            ..Security::default()
        };
        s.entries[0] = SecurityEntry {
            principal: principal::EVERYONE,
            id: 0,
            deny: false,
            access: access::READ | access::TRAVERSE,
            inherit: true,
        };
        s.entries[1] = SecurityEntry {
            principal: principal::GROUP,
            id: 3,
            deny: false,
            access: access::ALL,
            inherit: true,
        };
        let d = descriptor_of(&s);
        assert_eq!(
            to_sddl(&d, 0x17).unwrap(),
            "O:SYG:SYD:(A;OICI;0x1200a9;;;WD)(A;OICI;FA;;;BA)S:(ML;OICI;NW;;;ME)"
        );
        // Written back unchanged.
        assert_eq!(apply(&s, 0x17, &d), Ok(s));
        // A user's file: the owner shares it, Users read; app packages and
        // inherit-only entries do not reach storage.
        let mut file = Security {
            owner: 1001,
            count: 1,
            ..Security::default()
        };
        file.entries[0] = SecurityEntry {
            principal: principal::USER,
            id: 1001,
            deny: false,
            access: access::ALL,
            inherit: true,
        };
        let me = Sid::user(1001).text();
        let wanted = from_sddl(&format!(
            "D:P(A;;FA;;;{me})(A;;FR;;;AU)(A;;FR;;;BU)(A;;FR;;;AC)(A;OICIIO;GA;;;CO)"
        ))
        .unwrap();
        let out = apply(&file, info::DACL, &wanted).unwrap();
        assert_eq!(out.count, 2);
        assert_eq!(out.entries[1].principal, principal::GROUP);
        assert_eq!(out.entries[1].access, access::READ);
        assert_eq!(
            to_sddl(&descriptor_of(&out), 0x7).unwrap(),
            format!("O:{me}G:{me}D:(A;;FA;;;{me})(A;;FR;;;BU)")
        );
        // Administrators may own a file (what an elevated program creates
        // is theirs); storage decides whether the caller may make it so.
        let admins = apply(&file, info::OWNER, &from_sddl("O:BA").unwrap()).unwrap();
        assert_eq!(
            admins.owner,
            door_service_protocol::files::ADMINISTRATORS_OWNER
        );
        assert!(
            to_sddl(&descriptor_of(&admins), 0x1)
                .unwrap()
                .starts_with("O:BA")
        );
        assert_eq!(
            apply(&file, info::OWNER, &from_sddl("O:WD").unwrap()),
            Err(code::INVALID_OWNER)
        );
        let five = from_sddl(&format!(
            "D:(A;;FR;;;{me})(D;;FW;;;BU)(A;;FR;;;BA)(A;;FR;;;SY)(A;;FR;;;WD)"
        ))
        .unwrap();
        assert_eq!(apply(&file, info::DACL, &five), Err(error::NOT_SUPPORTED));
        assert_eq!(
            to_sddl(&key_descriptor(true, 1001), 0x17).unwrap(),
            "O:BAG:SYD:PAI(A;CI;KR;;;BU)(A;CI;KA;;;BA)(A;CI;KA;;;SY)S:NO_ACCESS_CONTROL"
        );
        assert_eq!(
            to_sddl(&key_descriptor(false, 1001), 0x17).unwrap(),
            format!("O:SYG:SYD:P(A;OICI;KA;;;{me})(A;OICI;KA;;;SY)S:AINO_ACCESS_CONTROL")
        );
    }
}
