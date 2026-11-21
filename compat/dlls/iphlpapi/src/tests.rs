use super::*;
use door_service_protocol::net::{Config, Dhcp};

const BASE: u64 = 0x0000_0200_0000_0000;

fn status() -> Status {
    Status {
        present: true,
        link: true,
        enabled: true,
        mac: [0x52, 0x54, 0x00, 0xd0, 0x05, 0x01],
        dhcp: Dhcp::Bound,
        address: [10, 0, 2, 15],
        prefix: 24,
        gateway: [10, 0, 2, 2],
        dns: [[10, 0, 2, 3], [0; 4]],
        lease_seconds: 86_000,
        frames_in: 7,
        frames_out: 9,
        bytes_in: 1000,
        bytes_out: 2000,
        config: Config::default(),
    }
}
fn u32_at(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(b[at..at + 4].try_into().unwrap())
}
fn u64_at(b: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(b[at..at + 8].try_into().unwrap())
}
/// The blob offset a stored pointer refers to.
fn target(b: &[u8], at: usize) -> Option<usize> {
    let p = u64_at(b, at);
    (p != 0).then(|| {
        assert!(
            p >= BASE && p < BASE + b.len() as u64,
            "pointer {p:#x} outside the buffer"
        );
        (p - BASE) as usize
    })
}
fn wide_at(b: &[u8], at: usize) -> String {
    let mut units = Vec::new();
    let mut i = at;
    loop {
        let u = u16::from_le_bytes([b[i], b[i + 1]]);
        if u == 0 {
            break;
        }
        units.push(u);
        i += 2;
    }
    String::from_utf16(&units).unwrap()
}
fn narrow_at(b: &[u8], at: usize) -> String {
    let end = b[at..].iter().position(|c| *c == 0).unwrap();
    String::from_utf8(b[at..at + end].to_vec()).unwrap()
}
fn sockaddr_at(b: &[u8], at: usize) -> [u8; 4] {
    let sa = target(b, at).unwrap();
    assert_eq!(u32_at(b, at + 8), 16);
    assert_eq!(u16::from_le_bytes([b[sa], b[sa + 1]]), AF_INET);
    b[sa + 4..sa + 8].try_into().unwrap()
}

#[test]
fn interfaces_follow_windows_naming_and_order() {
    let list = interfaces(Some(&status()), None);
    assert_eq!(list.len(), 2);
    let (ethernet, loopback) = (&list[0], &list[1]);
    assert_eq!(
        (ethernet.alias.as_str(), ethernet.index, ethernet.metric),
        ("Ethernet", 2, 15)
    );
    assert_eq!(ethernet.name(), "ethernet_32768");
    assert_eq!(ethernet.luid(), 0x0006_0080_0000_0000);
    assert_eq!(loopback.name(), "loopback_0");
    assert_eq!(loopback.luid(), 0x0018_0000_0000_0000);
    assert_eq!(
        ethernet.guid_text(),
        "{6DD0542E-0E0D-4F4D-8100-525400D00501}"
    );
    assert!(loopback.guid_text().ends_with("-806E6F6E6963}"));
    // No adapter: only the loopback; a cable pulled: no address, down.
    assert_eq!(interfaces(None, None).len(), 1);
    let mut unplugged = status();
    unplugged.link = false;
    let down = &interfaces(Some(&unplugged), None)[0];
    assert!(!down.up && down.ipv4.is_none() && down.speed == u64::MAX && down.metric == 5);
}

#[test]
fn a_running_tunnel_is_an_adapter_of_its_own() {
    let mut t = door_service_protocol::net::vpn::Tunnel::new("Office");
    t.address = [10, 8, 0, 2];
    t.prefix = 24;
    t.dns = [[10, 8, 0, 1], [0; 4]];
    t.mtu = 1420;
    let mut list = interfaces(Some(&status()), None);
    add_tunnel(&mut list, &t);
    assert_eq!(list.len(), 3);
    assert!(list[2].loopback());
    let b = adapters_addresses(BASE, AF_UNSPEC, 0, &list).unwrap();
    let tunnel = target(&b, 8).unwrap();
    assert_eq!(u32_at(&b, tunnel + 4), TUNNEL_INDEX);
    assert_eq!(
        wide_at(&b, target(&b, tunnel + 64).unwrap()),
        "WireGuard Tunnel"
    );
    assert_eq!(wide_at(&b, target(&b, tunnel + 72).unwrap()), "Office");
    assert_eq!(u32_at(&b, tunnel + 88), 0, "no physical address");
    assert_eq!(u32_at(&b, tunnel + 96), 1420);
    assert_eq!(u32_at(&b, tunnel + 100), IF_TYPE_PROP_VIRTUAL);
    let unicast = target(&b, tunnel + 24).unwrap();
    assert_eq!(sockaddr_at(&b, unicast + 16), [10, 8, 0, 2]);
    let info = adapters_info(BASE, &list).unwrap();
    let second = target(&info, 0).unwrap();
    assert_eq!(u32_at(&info, second + 420), IF_TYPE_PROP_VIRTUAL);
}

#[test]
fn adapters_addresses_match_the_sdk_layout() {
    let list = interfaces(Some(&status()), None);
    let b = adapters_addresses(
        BASE,
        AF_UNSPEC,
        gaa::INCLUDE_PREFIX | gaa::INCLUDE_GATEWAYS,
        &list,
    )
    .unwrap();
    // Ethernet first.
    assert_eq!(u32_at(&b, 0), 448);
    assert_eq!(u32_at(&b, 4), 2);
    let next = target(&b, 8).unwrap();
    assert_eq!(narrow_at(&b, target(&b, 16).unwrap()), list[0].guid_text());
    let unicast = target(&b, 24).unwrap();
    assert_eq!(u32_at(&b, unicast), 64);
    assert_eq!(u32_at(&b, unicast + 4), 1);
    assert_eq!(sockaddr_at(&b, unicast + 16), [10, 0, 2, 15]);
    assert_eq!(
        (
            u32_at(&b, unicast + 32),
            u32_at(&b, unicast + 36),
            u32_at(&b, unicast + 40)
        ),
        (3, 3, 4)
    );
    assert_eq!(u32_at(&b, unicast + 44), 86_000);
    assert_eq!(b[unicast + 56], 24);
    assert!(target(&b, unicast + 8).is_none());
    let dns = target(&b, 48).unwrap();
    assert_eq!(u32_at(&b, dns), 32);
    assert_eq!(sockaddr_at(&b, dns + 16), [10, 0, 2, 3]);
    assert!(target(&b, dns + 8).is_none());
    assert_eq!(wide_at(&b, target(&b, 56).unwrap()), "");
    assert_eq!(
        wide_at(&b, target(&b, 64).unwrap()),
        "Red Hat VirtIO Ethernet Adapter"
    );
    assert_eq!(wide_at(&b, target(&b, 72).unwrap()), "Ethernet");
    assert_eq!(&b[80..86], &[0x52, 0x54, 0x00, 0xd0, 0x05, 0x01]);
    assert_eq!(u32_at(&b, 88), 6);
    assert_eq!(u32_at(&b, 92), 0xc5);
    assert_eq!(
        (u32_at(&b, 96), u32_at(&b, 100), u32_at(&b, 104)),
        (1500, 6, 1)
    );
    assert_eq!(
        (u32_at(&b, 112), u32_at(&b, 128), u32_at(&b, 112 + 14 * 4)),
        (2, 1, 0)
    );
    // Prefixes: network, host, broadcast, multicast, limited broadcast.
    let mut prefix = target(&b, 176);
    let mut seen = Vec::new();
    while let Some(p) = prefix {
        assert_eq!(u32_at(&b, p), 40);
        seen.push((sockaddr_at(&b, p + 16), u32_at(&b, p + 32)));
        prefix = target(&b, p + 8);
    }
    assert_eq!(
        seen,
        [
            ([10, 0, 2, 0], 24),
            ([10, 0, 2, 15], 32),
            ([10, 0, 2, 255], 32),
            ([224, 0, 0, 0], 4),
            ([255, 255, 255, 255], 32)
        ]
    );
    assert_eq!(u64_at(&b, 184), 10_000_000_000);
    let gateway = target(&b, 208).unwrap();
    assert_eq!(sockaddr_at(&b, gateway + 16), [10, 0, 2, 2]);
    assert_eq!(u32_at(&b, 216), 15);
    assert_eq!(u64_at(&b, 224), list[0].luid());
    assert_eq!(sockaddr_at(&b, 232), [10, 0, 2, 2]);
    assert_eq!((u32_at(&b, 248), u32_at(&b, 268)), (1, 1));
    // Then the loopback, last.
    assert_eq!(next, 448);
    assert_eq!(u32_at(&b, next + 4), 1);
    assert!(target(&b, next + 8).is_none());
    assert_eq!(
        wide_at(&b, target(&b, next + 72).unwrap()),
        "Loopback Pseudo-Interface 1"
    );
    assert_eq!(
        (
            u32_at(&b, next + 92),
            u32_at(&b, next + 96),
            u32_at(&b, next + 100)
        ),
        (0x81, u32::MAX, 24)
    );
    // The loopback lists ::1 first, then 127.0.0.1.
    let lo6 = target(&b, next + 24).unwrap();
    let sa = target(&b, lo6 + 16).unwrap();
    assert_eq!(u32_at(&b, lo6 + 24), 28);
    assert_eq!(u16::from_le_bytes([b[sa], b[sa + 1]]), AF_INET6);
    assert_eq!(b[sa + 8..sa + 24], LOOPBACK6);
    let lo = target(&b, lo6 + 8).unwrap();
    assert_eq!(sockaddr_at(&b, lo + 16), [127, 0, 0, 1]);
    assert_eq!(
        (u32_at(&b, lo + 4), u32_at(&b, lo + 32), b[lo + 56]),
        (0, 2, 8)
    );
    assert!(target(&b, next + 48).is_none());
    // An IPv6 query keeps only IPv6 addresses.
    let v6 = adapters_addresses(BASE, AF_INET6, 0, &list).unwrap();
    assert!(target(&v6, 24).is_none());
    let lo6 = target(&v6, 448 + 24).unwrap();
    assert!(target(&v6, lo6 + 8).is_none());
    assert_eq!(
        adapters_addresses(BASE, 99, 0, &list),
        Err(ERROR_INVALID_PARAMETER)
    );
    // Skip flags drop their lists.
    let bare = adapters_addresses(
        BASE,
        AF_INET,
        gaa::SKIP_UNICAST | gaa::SKIP_DNS_SERVER | gaa::SKIP_FRIENDLY_NAME,
        &list,
    )
    .unwrap();
    assert!(
        target(&bare, 24).is_none() && target(&bare, 48).is_none() && target(&bare, 176).is_none()
    );
    assert_eq!(wide_at(&bare, target(&bare, 72).unwrap()), "");
}

#[test]
fn legacy_adapter_info_and_network_params() {
    let list = interfaces(Some(&status()), None);
    let b = adapters_info(BASE, &list).unwrap();
    assert_eq!(b.len(), 704);
    assert_eq!(u64_at(&b, 0), 0);
    assert_eq!(narrow_at(&b, 12), list[0].guid_text());
    assert_eq!(narrow_at(&b, 272), "Red Hat VirtIO Ethernet Adapter");
    assert_eq!(
        (
            u32_at(&b, 404),
            u32_at(&b, 416),
            u32_at(&b, 420),
            u32_at(&b, 424)
        ),
        (6, 2, 6, 1)
    );
    assert_eq!(narrow_at(&b, 440 + 8), "10.0.2.15");
    assert_eq!(narrow_at(&b, 440 + 24), "255.255.255.0");
    assert_eq!(narrow_at(&b, 488 + 8), "10.0.2.2");
    assert_eq!(narrow_at(&b, 536 + 8), "10.0.2.2");
    assert_eq!(
        adapters_info(BASE, &interfaces(None, None)),
        Err(ERROR_NO_DATA)
    );

    let f = fixed_info(BASE, "DOORS-PC", &[[10, 0, 2, 3], [1, 1, 1, 1]]);
    assert_eq!(narrow_at(&f, 0), "DOORS-PC");
    assert_eq!(target(&f, 264), Some(272));
    assert_eq!(narrow_at(&f, 272 + 8), "10.0.2.3");
    let second = target(&f, 272).unwrap();
    assert_eq!(second, 600);
    assert_eq!(narrow_at(&f, second + 8), "1.1.1.1");
    assert_eq!((u32_at(&f, 320), u32_at(&f, 592)), (8, 1));
}

#[test]
fn interface_rows_and_tables() {
    let list = interfaces(Some(&status()), None);
    let row = if_row2(&list[0]);
    assert_eq!(row.len(), 1352);
    assert_eq!(u64_at(&row, 0), list[0].luid());
    assert_eq!(u32_at(&row, 8), 2);
    assert_eq!(wide_at(&row, 28), "Ethernet");
    assert_eq!(wide_at(&row, 542), "Red Hat VirtIO Ethernet Adapter");
    assert_eq!(
        (
            u32_at(&row, 1056),
            u32_at(&row, 1128),
            u32_at(&row, 1156),
            u32_at(&row, 1164)
        ),
        (6, 6, 1, 1)
    );
    assert_eq!(row[1152], 0x5);
    assert_eq!((u64_at(&row, 1208), u64_at(&row, 1280)), (1000, 2000));
    let table = if_table2(&list);
    assert_eq!(table.len(), 8 + 2 * 1352);
    let old = if_row(&list[0]);
    assert_eq!(old.len(), 860);
    assert!(wide_at(&old, 0).starts_with("\\DEVICE\\TCPIP_{"));
    assert_eq!(
        (u32_at(&old, 512), u32_at(&old, 524), u32_at(&old, 544)),
        (2, 10_000_000_000u64 as u32, 5)
    );
    assert_eq!(u32_at(&old, 600), 32);
    assert_eq!(if_table(&list).len(), 4 + 2 * 860);

    let addresses = ip_addr_table(&list, true);
    assert_eq!(u32_at(&addresses, 0), 2);
    assert_eq!(&addresses[4..8], &[10, 0, 2, 15]);
    assert_eq!(&addresses[28..32], &[127, 0, 0, 1]);
    assert_eq!(u16::from_le_bytes([addresses[26], addresses[27]]), 0x5);

    let routes = ip_forward_table(&list, true);
    let count = u32_at(&routes, 0) as usize;
    assert_eq!(routes.len(), 4 + count * 56);
    // The default route is first, through the gateway, indirect.
    assert_eq!(&routes[4..8], &[0; 4]);
    assert_eq!(&routes[16..20], &[10, 0, 2, 2]);
    assert_eq!(
        (
            u32_at(&routes, 4 + 20),
            u32_at(&routes, 4 + 24),
            u32_at(&routes, 4 + 36)
        ),
        (4, 3, 15)
    );
    assert_eq!(best_interface(&list, [8, 8, 8, 8]), Ok(2));
    assert_eq!(best_interface(&list, [127, 0, 0, 5]), Ok(1));
    assert_eq!(
        best_interface(&interfaces(None, None), [8, 8, 8, 8]),
        Err(ERROR_NETWORK_UNREACHABLE)
    );

    let unicast = unicast_row(&list[0]).unwrap();
    assert_eq!(&unicast[4..8], &[10, 0, 2, 15]);
    assert_eq!(
        (u32_at(&unicast, 40), unicast[60], u32_at(&unicast, 64)),
        (2, 24, 4)
    );
    let ip = ip_interface_row(&list[1]);
    assert_eq!(
        (u32_at(&ip, 16), u32_at(&ip, 148), u32_at(&ip, 152)),
        (1, 75, u32::MAX)
    );
    assert!(by_name(&list, "ETHERNET_32768").is_some() && by_alias(&list, "ethernet").is_some());
    assert!(by_luid(&list, 0x0018_0000_0000_0000).is_some_and(|i| i.index == 1));
}

#[test]
fn autoconfigured_ipv6_is_listed_before_ipv4() {
    use door_service_protocol::net::v6::Status6;
    let mut link_local = [0u8; 16];
    link_local[..2].copy_from_slice(&[0xfe, 0x80]);
    link_local[8..].copy_from_slice(&[0x50, 0x54, 0x00, 0xff, 0xfe, 0xd0, 0x05, 0x01]);
    let mut global = link_local;
    global[..2].copy_from_slice(&[0xfe, 0xc0]);
    let mut router = [0u8; 16];
    router[..2].copy_from_slice(&[0xfe, 0x80]);
    router[15] = 2;
    let v6 = Status6 {
        link_local,
        global,
        prefix: 64,
        router,
        dns: [[0; 16]; 2],
        scope: 4,
    };
    let list = interfaces(Some(&status()), Some(&v6));
    let b = adapters_addresses(BASE, AF_UNSPEC, gaa::INCLUDE_GATEWAYS, &list).unwrap();
    // IPV6_ENABLED and Ipv6IfIndex.
    assert_eq!(u32_at(&b, 92) & 0x100, 0x100);
    assert_eq!(u32_at(&b, 108), list[0].index);
    // Global (router advertisement origin), link-local with its scope id,
    // then the IPv4 address.
    let first = target(&b, 24).unwrap();
    let sa = target(&b, first + 16).unwrap();
    assert_eq!(
        (b[sa + 8..sa + 24].to_vec(), u32_at(&b, first + 32)),
        (global.to_vec(), 4)
    );
    let second = target(&b, first + 8).unwrap();
    let sa = target(&b, second + 16).unwrap();
    assert_eq!(b[sa + 8..sa + 24], link_local);
    assert_eq!(u32_at(&b, sa + 24), list[0].index);
    let third = target(&b, second + 8).unwrap();
    assert_eq!(sockaddr_at(&b, third + 16), [10, 0, 2, 15]);
    // The IPv6 router heads the gateway list.
    let g = target(&b, 208).unwrap();
    assert_eq!(u32_at(&b, g + 24), 28);
    let next = target(&b, g + 8).unwrap();
    assert_eq!(sockaddr_at(&b, next + 16), [10, 0, 2, 2]);
}

#[test]
fn statistics_and_neighbor_tables_match_the_sdk_layout() {
    use door_service_protocol::net::stats::{Family, NEIGHBOR_DYNAMIC, NEIGHBOR_STATIC, Neighbor};
    let list = interfaces(Some(&status()), None);
    let mut f = Family {
        ip_in_receives: 10,
        ip_out_requests: 11,
        tcp_active_opens: 3,
        tcp_num_conns: 2,
        udp_no_ports: 4,
        icmp_in_msgs: 5,
        icmp_out_msgs: 6,
        ..Family::default()
    };
    f.icmp_in[0] = 2; // echo replies
    f.icmp_out[8] = 6; // echos
    let ip = ip_stats(&f, &list, false);
    assert_eq!(ip.len(), 92);
    assert_eq!(
        (u32_at(&ip, 0), u32_at(&ip, 4), u32_at(&ip, 8)),
        (2, 128, 10)
    );
    assert_eq!((u32_at(&ip, 36), u32_at(&ip, 52)), (11, 60));
    // Two IPv4 interfaces and addresses (Ethernet, loopback).
    assert_eq!((u32_at(&ip, 80), u32_at(&ip, 84)), (2, 2));
    let tcp = tcp_stats(&f);
    assert_eq!(tcp.len(), 60);
    assert_eq!(
        (u32_at(&tcp, 0), u32_at(&tcp, 4), u32_at(&tcp, 8)),
        (4, 5, u32::MAX)
    );
    assert_eq!((u32_at(&tcp, 16), u32_at(&tcp, 56)), (3, 2));
    let udp = udp_stats(&f);
    assert_eq!((udp.len(), u32_at(&udp, 4)), (20, 4));
    let icmp = icmp_stats(&f);
    assert_eq!(icmp.len(), 104);
    // Received: messages, ..., echo replies at index 8; sent: echos at 7.
    assert_eq!((u32_at(&icmp, 0), u32_at(&icmp, 32)), (5, 2));
    assert_eq!((u32_at(&icmp, 52), u32_at(&icmp, 52 + 28)), (6, 6));
    let ex = icmp_stats_ex(&f, false);
    assert_eq!(ex.len(), 2064);
    assert_eq!((u32_at(&ex, 8), u32_at(&ex, 1032 + 8 + 8 * 4)), (2, 6));

    let gateway = Neighbor::v4([10, 0, 2, 2], [0x52, 0x55, 10, 0, 2, 2], NEIGHBOR_DYNAMIC);
    let broadcast = Neighbor::v4([10, 0, 2, 255], [0xff; 6], NEIGHBOR_STATIC);
    let mut router = [0u8; 16];
    router[..2].copy_from_slice(&[0xfe, 0x80]);
    router[15] = 2;
    let six = Neighbor {
        address: router,
        mac: [0x52, 0x56, 0, 0, 0, 2],
        kind: NEIGHBOR_DYNAMIC,
        router: true,
    };
    let table = ip_net_table(&list, &[broadcast, six, gateway], true);
    // IPv4 rows only, by address.
    assert_eq!((table.len(), u32_at(&table, 0)), (4 + 2 * 24, 2));
    assert_eq!(u32_at(&table, 4), ETHERNET_INDEX);
    assert_eq!(u32_at(&table, 8), 6);
    assert_eq!(&table[12..18], &[0x52, 0x55, 10, 0, 2, 2]);
    assert_eq!(&table[20..24], &[10, 0, 2, 2]);
    assert_eq!((u32_at(&table, 24), u32_at(&table, 48)), (3, 4));
    let ethernet = list.iter().find(|i| !i.loopback()).unwrap();
    let row = ip_net_row2(&six, ethernet);
    assert_eq!(u16::from_le_bytes([row[0], row[1]]), AF_INET6);
    assert_eq!(&row[8..24], &router);
    assert_eq!(u32_at(&row, 24), ETHERNET_INDEX); // link-local scope id
    assert_eq!(
        (u32_at(&row, 28), u64_at(&row, 32)),
        (ETHERNET_INDEX, ethernet.luid())
    );
    assert_eq!((u32_at(&row, 72), u32_at(&row, 76), row[80]), (6, 5, 1));
    let row = ip_net_row2(&broadcast, ethernet);
    assert_eq!(
        (u16::from_le_bytes([row[0], row[1]]), &row[4..8]),
        (AF_INET, &[10u8, 0, 2, 255][..])
    );
    assert_eq!((u32_at(&row, 76), row[80]), (6, 0));
}

/// P17: the 32-bit layouts (iptypes.h and IPExport.h built for x86).
#[test]
fn x86_layouts_match_the_sdk() {
    assert_eq!(layout::size_at(&ADAPTER_ADDRESSES, 8), 448);
    assert_eq!(layout::size_at(&ADAPTER_ADDRESSES, 4), 376);
    assert_eq!(layout::size_at(&UNICAST_ADDRESS, 4), 48);
    assert_eq!(layout::size_at(&SERVER_ADDRESS, 4), 24);
    assert_eq!(layout::size_at(&ADAPTER_PREFIX, 4), 24);
    assert_eq!(layout::size_at(&ADDR_STRING, 4), 40);
    assert_eq!(layout::size_at(&ADAPTER_INFO, 8), 704);
    assert_eq!(layout::size_at(&ADAPTER_INFO, 4), 648);
    assert_eq!(layout::size_at(&FIXED_INFO, 8), 600);
    assert_eq!(layout::size_at(&FIXED_INFO, 4), 584);
    assert_eq!(layout::offset_at(&ADAPTER_ADDRESSES, aa::FRIENDLY, 4), 40);
    assert_eq!(layout::offset_at(&ADAPTER_ADDRESSES, aa::LUID, 4), 176);
    let list = interfaces(Some(&status()), None);
    let base = 0x0040_0000u64;
    let b = adapters_addresses_at(base, AF_UNSPEC, 0, &list, 4).unwrap();
    assert_eq!(u32_at(&b, 0), 376);
    // Next: a 4-byte pointer to the second adapter.
    assert_eq!(u32_at(&b, 8), base as u32 + 376);
    let unicast = (u32_at(&b, 16) - base as u32) as usize;
    assert_eq!(u32_at(&b, unicast), 48);
    let sockaddr = (u32_at(&b, unicast + 12) - base as u32) as usize;
    assert_eq!(u32_at(&b, unicast + 16), 16);
    assert_eq!(b[sockaddr + 4..sockaddr + 8], [10, 0, 2, 15]);
    let info = adapters_info_at(base, &list, 4).unwrap();
    assert_eq!(
        &info[8..8 + 38],
        interfaces(Some(&status()), None)[0].guid_text().as_bytes()
    );
    let f = fixed_info_at(base, "DOORS-PC", &[[10, 0, 2, 3]], 4);
    assert_eq!(u32_at(&f, 264), base as u32 + 268);
    assert_eq!(&f[268 + 4..268 + 12], b"10.0.2.3");
    assert_eq!(u32_at(&f, 580), 1);
}
