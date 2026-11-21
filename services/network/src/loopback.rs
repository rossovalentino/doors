//! The loopback interface, as Windows' Loopback Pseudo-Interface: 127.0.0.0/8
//! and ::1, and this host's own adapter addresses, never reach the wire. The
//! stack sends to them like to any neighbour; the device answers its address
//! resolution for them (ARP, IPv6 neighbour solicitation) with the adapter's
//! own MAC and hands every frame addressed to that MAC straight back to the
//! stack. Duplicate-address probes (unspecified source) are not answered.
use alloc::{vec, vec::Vec};

/// 127.0.0.1/8 and ::1/128 on the interface.
pub const V4: [u8; 4] = [127, 0, 0, 1];
pub const V4_PREFIX: u8 = 8;
pub const V6: [u8; 16] = {
    let mut a = [0u8; 16];
    a[15] = 1;
    a
};
const ETHERTYPE_ARP: u16 = 0x0806;
const ETHERTYPE_IPV6: u16 = 0x86dd;

/// What a frame the stack sends to `mac`'s own interface becomes: a frame
/// to hand back to the stack (the frame itself, or the answer to its
/// address query), or `None` for the wire.
pub fn route(frame: &[u8], mac: [u8; 6]) -> Option<Vec<u8>> {
    if frame.len() < 14 {
        return None;
    }
    if frame[..6] == mac {
        return Some(frame.to_vec());
    }
    match u16::from_be_bytes([frame[12], frame[13]]) {
        ETHERTYPE_ARP => arp_answer(&frame[14..], mac),
        ETHERTYPE_IPV6 => neighbor_answer(&frame[14..], mac),
        _ => None,
    }
}
fn ethernet(mac: [u8; 6], ethertype: u16, payload: &[u8]) -> Vec<u8> {
    let mut f = vec![0u8; 14 + payload.len()];
    f[..6].copy_from_slice(&mac);
    f[6..12].copy_from_slice(&mac);
    f[12..14].copy_from_slice(&ethertype.to_be_bytes());
    f[14..].copy_from_slice(payload);
    f
}
/// The stack's neighbour entries for its own addresses, before it asks:
/// an ARP reply and a neighbour advertisement each, as if from the address
/// to itself. smoltcp sends one resolution request a second and forgets an
/// entry after a minute, so the device refreshes these on a timer.
pub fn seed(mac: [u8; 6], v4: &[[u8; 4]], v6: &[[u8; 16]]) -> Vec<Vec<u8>> {
    let mut frames = Vec::new();
    for a in v4.iter().filter(|a| **a != [0; 4]) {
        frames.push(arp_reply(mac, *a, *a));
    }
    for a in v6.iter().filter(|a| **a != [0; 16]) {
        frames.push(advertisement(mac, *a, *a));
    }
    frames
}
/// An ARP request for a loopback address, or for the asker's own address.
fn arp_answer(arp: &[u8], mac: [u8; 6]) -> Option<Vec<u8>> {
    // Ethernet/IPv4, request.
    if arp.len() < 28 || arp[..8] != [0, 1, 8, 0, 6, 4, 0, 1] {
        return None;
    }
    let sender_ip: [u8; 4] = arp[14..18].try_into().unwrap();
    let target_ip: [u8; 4] = arp[24..28].try_into().unwrap();
    if target_ip[0] != 127 && (target_ip != sender_ip || sender_ip == [0; 4]) {
        return None;
    }
    Some(arp_reply(mac, target_ip, sender_ip))
}
/// "`address` is at `mac`", to `to`.
fn arp_reply(mac: [u8; 6], address: [u8; 4], to: [u8; 4]) -> Vec<u8> {
    let mut reply = [0u8; 28];
    reply[..8].copy_from_slice(&[0, 1, 8, 0, 6, 4, 0, 2]);
    reply[8..14].copy_from_slice(&mac);
    reply[14..18].copy_from_slice(&address);
    reply[18..24].copy_from_slice(&mac);
    reply[24..28].copy_from_slice(&to);
    ethernet(mac, ETHERTYPE_ARP, &reply)
}
/// A neighbour solicitation for ::1, or for the asker's own address.
fn neighbor_answer(ip: &[u8], mac: [u8; 6]) -> Option<Vec<u8>> {
    // IPv6, ICMPv6, at least the solicitation's 24 bytes.
    if ip.len() < 40 + 24 || ip[0] >> 4 != 6 || ip[6] != 58 || ip[40] != 135 {
        return None;
    }
    let source: [u8; 16] = ip[8..24].try_into().unwrap();
    let target: [u8; 16] = ip[48..64].try_into().unwrap();
    if target != V6 && (target != source || source == [0; 16]) {
        return None;
    }
    Some(advertisement(mac, target, source))
}
/// A neighbour advertisement, solicited and override, of `target` at
/// `mac`, to `source`.
fn advertisement(mac: [u8; 6], target: [u8; 16], source: [u8; 16]) -> Vec<u8> {
    neighbor_advertisement(mac, mac, target, source)
}
/// A neighbour advertisement of `target` at `link` (the frame from
/// there), to `source` at `to`.
pub fn neighbor_advertisement(
    to: [u8; 6],
    link: [u8; 6],
    target: [u8; 16],
    source: [u8; 16],
) -> Vec<u8> {
    let mut icmp = [0u8; 32];
    icmp[0] = 136;
    icmp[4] = 0x60;
    icmp[8..24].copy_from_slice(&target);
    icmp[24] = 2;
    icmp[25] = 1;
    icmp[26..32].copy_from_slice(&link);
    let mut pseudo = [0u8; 40];
    pseudo[..16].copy_from_slice(&target);
    pseudo[16..32].copy_from_slice(&source);
    pseudo[32..36].copy_from_slice(&(icmp.len() as u32).to_be_bytes());
    pseudo[39] = 58;
    let sum = crate::internet_checksum(&[&pseudo, &icmp]);
    icmp[2..4].copy_from_slice(&sum.to_be_bytes());
    let mut packet = vec![0u8; 40 + icmp.len()];
    packet[0] = 0x60;
    packet[4..6].copy_from_slice(&(icmp.len() as u16).to_be_bytes());
    packet[6] = 58;
    packet[7] = 255;
    packet[8..24].copy_from_slice(&target);
    packet[24..40].copy_from_slice(&source);
    packet[40..].copy_from_slice(&icmp);
    let mut frame = ethernet(to, ETHERTYPE_IPV6, &packet);
    frame[6..12].copy_from_slice(&link);
    frame
}

#[cfg(test)]
mod tests {
    use super::*;
    const MAC: [u8; 6] = [0x52, 0x54, 0, 0x12, 0x34, 0x56];
    fn arp_request(sender: [u8; 4], target: [u8; 4]) -> Vec<u8> {
        let mut arp = vec![0, 1, 8, 0, 6, 4, 0, 1];
        arp.extend(MAC);
        arp.extend(sender);
        arp.extend([0; 6]);
        arp.extend(target);
        let mut f = vec![0xff; 6];
        f.extend(MAC);
        f.extend(ETHERTYPE_ARP.to_be_bytes());
        f.extend(arp);
        f
    }

    #[test]
    fn loopback_and_self_resolution_is_answered_locally() {
        let reply = route(&arp_request(V4, [127, 0, 0, 1]), MAC).unwrap();
        assert_eq!(&reply[..6], &MAC);
        assert_eq!(&reply[20..22], &[0, 2]);
        assert_eq!(&reply[22..28], &MAC);
        assert_eq!(&reply[28..32], &[127, 0, 0, 1]);
        assert_eq!(&reply[38..42], &V4);
        // Asking for its own adapter address, too; the gateway goes out.
        let own = [10, 0, 2, 15];
        assert!(route(&arp_request(own, own), MAC).is_some());
        assert!(route(&arp_request(own, [10, 0, 2, 2]), MAC).is_none());
        assert!(route(&arp_request([0; 4], [0; 4]), MAC).is_none());
        // A frame to this host's MAC comes back as it is.
        let mut frame = vec![0u8; 60];
        frame[..6].copy_from_slice(&MAC);
        assert_eq!(route(&frame, MAC), Some(frame.clone()));
        frame[0] = 0x02;
        assert_eq!(route(&frame, MAC), None);
    }

    #[test]
    fn a_solicitation_for_the_loopback_address_is_advertised() {
        let mut ip = vec![0u8; 64];
        ip[0] = 0x60;
        ip[6] = 58;
        ip[7] = 255;
        ip[8..24].copy_from_slice(&V6);
        ip[40] = 135;
        ip[48..64].copy_from_slice(&V6);
        let mut frame = vec![0x33, 0x33, 0xff, 0, 0, 1];
        frame.extend(MAC);
        frame.extend(ETHERTYPE_IPV6.to_be_bytes());
        frame.extend(&ip);
        let reply = route(&frame, MAC).unwrap();
        let packet = &reply[14..];
        assert_eq!(packet[40], 136);
        assert_eq!(&packet[48..64], &V6);
        assert_eq!(&packet[66..72], &MAC);
        // The checksum verifies: summing with it gives zero.
        let mut pseudo = [0u8; 40];
        pseudo[..16].copy_from_slice(&packet[8..24]);
        pseudo[16..32].copy_from_slice(&packet[24..40]);
        pseudo[32..36].copy_from_slice(&32u32.to_be_bytes());
        pseudo[39] = 58;
        assert_eq!(crate::internet_checksum(&[&pseudo, &packet[40..]]), 0);
        // Duplicate-address detection (unspecified source) is left alone.
        let mut dad = frame.clone();
        dad[14 + 8..14 + 24].fill(0);
        let mut global = [0u8; 16];
        global[..2].copy_from_slice(&[0x20, 0x01]);
        dad[14 + 48..14 + 64].copy_from_slice(&global);
        assert!(route(&dad, MAC).is_none());
    }
}
