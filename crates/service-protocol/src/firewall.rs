//! Windows Defender Firewall's store, kept by Init where Windows keeps it:
//! `HKLM\System\CurrentControlSet\Services\SharedAccess\Parameters\
//! FirewallPolicy` (the `FirewallRules` values in their "v2.33|..." form,
//! and one key of switches per profile). Init hands the policy to
//! door.network (`net::firewall`) when the network starts and after every
//! change. Administrators (elevated `netsh advfirewall`) write the registry
//! themselves and ask Init to RELOAD it; the window server changes it
//! through Init once User Account Control approved (the Security Alert,
//! the Settings firewall page).
use crate::net::firewall::profile as bit;

/// Re-read the registry and hand the policy to door.network (empty body).
pub const RELOAD: u32 = 0x7030;
/// The window server, after consent: `[kind u8]...` (`change`).
pub const CHANGE: u32 = 0x7031;
/// The Init operations' range.
pub const FIRST: u32 = 0x7030;
pub const LAST: u32 = 0x703f;
pub mod change {
    /// `[PROFILES][profile bits u8][profile byte u8]`: the switches of
    /// the profiles named (`door_firewall::profile` bits).
    pub const PROFILES: u8 = 1;
    /// `[PROGRAM][allow u8][profile bits u8][path UTF-8]`: inbound TCP and
    /// UDP rules for the program, as the Security Alert writes them.
    pub const PROGRAM: u8 = 2;
    /// `[RESTORE]`: Windows' defaults (no rules, every profile on and
    /// blocking unsolicited inbound connections).
    pub const RESTORE: u8 = 3;
}
pub const POLICY_KEY: &str =
    r"System\CurrentControlSet\Services\SharedAccess\Parameters\FirewallPolicy";
pub const RULES: &str = "FirewallRules";
/// Domain, private ("Standard"), public: Windows' key names.
pub const PROFILE_KEYS: [&str; 3] = ["DomainProfile", "StandardProfile", "PublicProfile"];
/// The DWORD switches of a profile key, in `profile_values` order.
pub const VALUES: [&str; 4] = [
    "EnableFirewall",
    "DefaultInboundAction",
    "DefaultOutboundAction",
    "DoNotAllowExceptions",
];
/// A profile byte (`net::firewall::profile`) from its four values; an
/// absent one takes Windows' default (on, inbound blocked, outbound
/// allowed, exceptions allowed).
pub fn profile_byte(values: [Option<u32>; 4]) -> u8 {
    let [enable, inbound, outbound, shields] = values;
    let mut b = 0;
    if enable.unwrap_or(1) != 0 {
        b |= bit::ON;
    }
    if inbound.unwrap_or(1) == 0 {
        b |= bit::INBOUND_ALLOW;
    }
    if outbound.unwrap_or(0) != 0 {
        b |= bit::OUTBOUND_BLOCK;
    }
    if shields.unwrap_or(0) != 0 {
        b |= bit::SHIELDS_UP;
    }
    b
}
/// The four values for a profile byte (`VALUES` order).
pub fn profile_values(byte: u8) -> [u32; 4] {
    [
        u32::from(byte & bit::ON != 0),
        u32::from(byte & bit::INBOUND_ALLOW == 0),
        u32::from(byte & bit::OUTBOUND_BLOCK != 0),
        u32::from(byte & bit::SHIELDS_UP != 0),
    ]
}
/// The value name of a program rule: a GUID drawn from the path (case
/// folded) and the protocol, so allowing a program again replaces its
/// rules.
pub fn program_rule_id(path: &str, protocol: u8) -> [u8; 38] {
    let mut hash = [0xcbf2_9ce4_8422_2325u64, 0x8422_2325_cbf2_9ce4];
    for (i, h) in hash.iter_mut().enumerate() {
        for b in path
            .bytes()
            .map(|b| b.to_ascii_lowercase())
            .chain([protocol, i as u8])
        {
            *h = (*h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    let mut raw = [0u8; 16];
    raw[..8].copy_from_slice(&hash[0].to_be_bytes());
    raw[8..].copy_from_slice(&hash[1].to_be_bytes());
    let mut out = [0u8; 38];
    out[0] = b'{';
    out[37] = b'}';
    let mut at = 1;
    for (i, byte) in raw.iter().enumerate() {
        if matches!(i, 4 | 6 | 8 | 10) {
            out[at] = b'-';
            at += 1;
        }
        for nibble in [byte >> 4, byte & 0xf] {
            out[at] = b"0123456789ABCDEF"[usize::from(nibble)];
            at += 1;
        }
    }
    out
}
/// The registry form of a program rule (`change::PROGRAM`), named after
/// the program's file, into `out`: its length, or None when it does not
/// fit.
pub fn program_rule(
    allow: bool,
    protocol: u8,
    profiles: u8,
    path: &str,
    out: &mut [u8],
) -> Option<usize> {
    struct Out<'a> {
        bytes: &'a mut [u8],
        len: usize,
    }
    impl core::fmt::Write for Out<'_> {
        fn write_str(&mut self, s: &str) -> core::fmt::Result {
            let end = self.len + s.len();
            self.bytes
                .get_mut(self.len..end)
                .ok_or(core::fmt::Error)?
                .copy_from_slice(s.as_bytes());
            self.len = end;
            Ok(())
        }
    }
    if path.is_empty() || path.contains('|') {
        return None;
    }
    let file = path.rsplit(['\\', '/']).next().unwrap_or(path);
    let mut w = Out { bytes: out, len: 0 };
    use core::fmt::Write;
    write!(
        w,
        "v2.33|Action={}|Active=TRUE|Dir=In|Protocol={protocol}|",
        if allow { "Allow" } else { "Block" }
    )
    .ok()?;
    // door_firewall's profile bits; all three are written as none.
    for (b, name) in [(1, "Domain"), (2, "Private"), (4, "Public")] {
        if profiles & b != 0 && profiles & 7 != 7 {
            write!(w, "Profile={name}|").ok()?;
        }
    }
    write!(w, "App={path}|Name={file}|Desc={file}|Defer=User|").ok()?;
    Some(w.len)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn profile_values_round_trip() {
        assert_eq!(profile_byte([None; 4]), bit::DEFAULT);
        for byte in 0..16u8 {
            assert_eq!(profile_byte(profile_values(byte).map(Some)), byte);
        }
        assert_eq!(profile_values(bit::DEFAULT), [1, 1, 0, 0]);
    }
    #[test]
    fn program_rules() {
        let id = program_rule_id(r"C:\Users\1001\app.exe", 6);
        assert_eq!(id[0], b'{');
        assert_eq!(id[37], b'}');
        assert_eq!(&id[9..10], b"-");
        assert_eq!(id, program_rule_id(r"c:\users\1001\APP.exe", 6));
        assert_ne!(id, program_rule_id(r"C:\Users\1001\app.exe", 17));
        let mut out = [0u8; 300];
        let n = program_rule(true, 6, 4, r"C:\Users\1001\app.exe", &mut out).unwrap();
        assert_eq!(
            &out[..n],
            br"v2.33|Action=Allow|Active=TRUE|Dir=In|Protocol=6|Profile=Public|App=C:\Users\1001\app.exe|Name=app.exe|Desc=app.exe|Defer=User|"
        );
        let n = program_rule(false, 17, 7, r"C:\a.exe", &mut out).unwrap();
        assert_eq!(
            &out[..n],
            br"v2.33|Action=Block|Active=TRUE|Dir=In|Protocol=17|App=C:\a.exe|Name=a.exe|Desc=a.exe|Defer=User|"
        );
        assert!(program_rule(true, 6, 4, "a|b", &mut out).is_none());
        assert!(program_rule(true, 6, 4, r"C:\a.exe", &mut out[..20]).is_none());
    }
}
