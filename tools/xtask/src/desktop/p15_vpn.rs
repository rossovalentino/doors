//! P15 VPN-A: a WireGuard connection from the guest to an independent
//! peer on this host. The peer is boringtun (Cloudflare's implementation)
//! listening on a UDP port the guest reaches at 10.0.2.2, with a small IP
//! stack (smoltcp) behind it at 10.8.0.1: TCP and UDP echo on port 7, DNS
//! for `vpn.doors.test`, and ICMP to ping the guest's tunnel address.
//!
//! In Terminal the run types two wg-quick files (`office.conf` routing
//! everything, so the kill-switch is on, and `badkey.conf` naming a server
//! key nobody holds), imports and dials them with `wireguard
//! /installtunnelservice` and `rasdial`, runs `vpnprobe.exe` (an MSVC-built
//! Winsock program) through the tunnel and again after hanging up, pings
//! the guest through the tunnel from the host (unanswered until an elevated
//! `netsh advfirewall` rule allows ICMPv4 in), and expects Remote Access
//! error 809 from the wrong key.
use super::p15_connect::{Probe, build, doorsh, open_admin_terminal, run_probe, stage};
use super::*;
use boringtun::noise::{Tunn, TunnResult};
use boringtun::x25519::{PublicKey, StaticSecret};
use smoltcp::{
    iface::{Config as IfaceConfig, Interface, SocketSet},
    phy::{Device, DeviceCapabilities, Medium, RxToken, TxToken},
    socket::{icmp, tcp, udp},
    time::Instant as NetInstant,
    wire::{HardwareAddress, Icmpv4Packet, Icmpv4Repr, IpAddress, IpCidr},
};
use std::{
    collections::VecDeque,
    io::{Read, Write},
    net::{Shutdown, TcpListener, UdpSocket},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU32, Ordering},
    },
};

const VPNPROBE: Probe = Probe {
    name: "vpnprobe",
    directory: "tests/fixtures/windows-net",
    libraries: &["kernel32.lib", "WS2_32.Lib"],
    modules: &["kernel32.dll", "ws2_32.dll"],
};
const RESOLUTION: &str = "1280x720";
const TUNNEL_STEPS: [&str; 6] = [
    "startup",
    "dns",
    "tcp-echo",
    "udp-echo",
    "udp6-echo",
    "all-tunneled",
];
const DIRECT_STEPS: [&str; 3] = ["startup", "direct", "tunnel-gone"];

/// A key for this run: SHA-256 of a label and the run id, clamped.
fn run_key(label: &str, run: &str) -> [u8; 32] {
    use sha2::Digest;
    let digest: [u8; 32] = sha2::Sha256::digest(format!("{label}:{run}").as_bytes()).into();
    door_wireguard::private_key(digest)
}

/// The host side of the tunnel.
struct Peer {
    port: u16,
    stop: Arc<AtomicBool>,
    ping: Arc<AtomicBool>,
    replies: Arc<AtomicU32>,
    log: Arc<Mutex<Vec<String>>>,
}
impl Peer {
    /// Ping 10.8.0.2 through the tunnel until it answers or `seconds` pass.
    fn ping(&self, seconds: u64) -> Result<u32, String> {
        let before = self.replies.load(Ordering::Relaxed);
        self.ping.store(true, Ordering::Relaxed);
        let deadline = Instant::now() + Duration::from_secs(seconds);
        while Instant::now() < deadline {
            let now = self.replies.load(Ordering::Relaxed);
            if now >= before + 3 {
                self.ping.store(false, Ordering::Relaxed);
                return Ok(now - before);
            }
            thread::sleep(Duration::from_millis(100));
        }
        self.ping.store(false, Ordering::Relaxed);
        Err(format!(
            "the guest answered {} of the host's pings through the tunnel",
            self.replies.load(Ordering::Relaxed) - before
        ))
    }
}
/// Packets between boringtun and smoltcp.
#[derive(Default)]
struct Wire {
    inbound: VecDeque<Vec<u8>>,
    outbound: Vec<Vec<u8>>,
}
struct WireRx(Vec<u8>);
struct WireTx<'a>(&'a mut Vec<Vec<u8>>);
impl RxToken for WireRx {
    fn consume<R, F: FnOnce(&[u8]) -> R>(self, f: F) -> R {
        f(&self.0)
    }
}
impl TxToken for WireTx<'_> {
    fn consume<R, F: FnOnce(&mut [u8]) -> R>(self, len: usize, f: F) -> R {
        let mut b = vec![0u8; len];
        let r = f(&mut b);
        self.0.push(b);
        r
    }
}
impl Device for Wire {
    type RxToken<'a> = WireRx;
    type TxToken<'a> = WireTx<'a>;
    fn receive(&mut self, _: NetInstant) -> Option<(WireRx, WireTx<'_>)> {
        let p = self.inbound.pop_front()?;
        Some((WireRx(p), WireTx(&mut self.outbound)))
    }
    fn transmit(&mut self, _: NetInstant) -> Option<WireTx<'_>> {
        Some(WireTx(&mut self.outbound))
    }
    fn capabilities(&self) -> DeviceCapabilities {
        let mut c = DeviceCapabilities::default();
        c.medium = Medium::Ip;
        c.max_transmission_unit = 1420;
        c
    }
}
/// A DNS answer from the tunnel's server: vpn.doors.test A 10.8.0.1,
/// NXDOMAIN for any other name.
fn dns_answer(query: &[u8]) -> Option<Vec<u8>> {
    if query.len() < 12 || u16::from_be_bytes([query[4], query[5]]) != 1 {
        return None;
    }
    let mut at = 12;
    let mut labels = Vec::new();
    loop {
        let n = usize::from(*query.get(at)?);
        at += 1;
        if n == 0 {
            break;
        }
        labels.push(String::from_utf8_lossy(query.get(at..at + n)?).to_ascii_lowercase());
        at += n;
    }
    let qtype = u16::from_be_bytes([*query.get(at)?, *query.get(at + 1)?]);
    at += 4;
    let name = labels.join(".");
    let known = name == "vpn.doors.test";
    let mut r = query.get(..at)?.to_vec();
    r[2] = 0x84 | (query[2] & 0x01);
    r[3] = 0x80 | if known { 0 } else { 3 };
    let answers = u16::from(known && qtype == 1);
    r[6..8].copy_from_slice(&answers.to_be_bytes());
    r[8..12].fill(0);
    if answers == 1 {
        r.extend_from_slice(&[0xC0, 0x0C, 0, 1, 0, 1, 0, 0, 0, 60, 0, 4, 10, 8, 0, 1]);
    }
    Some(r)
}
fn note(log: &Mutex<Vec<String>>, line: String) {
    if let Ok(mut l) = log.lock() {
        l.push(line);
    }
}
fn serve_peer(server_private: [u8; 32], client_public: [u8; 32]) -> Result<Peer, String> {
    let socket = UdpSocket::bind("127.0.0.1:0").map_err(|e| e.to_string())?;
    socket
        .set_read_timeout(Some(Duration::from_millis(5)))
        .map_err(|e| e.to_string())?;
    let peer = Peer {
        port: socket.local_addr().map_err(|e| e.to_string())?.port(),
        stop: Arc::new(AtomicBool::new(false)),
        ping: Arc::new(AtomicBool::new(false)),
        replies: Arc::new(AtomicU32::new(0)),
        log: Arc::new(Mutex::new(Vec::new())),
    };
    let (stop, ping, replies, log) = (
        peer.stop.clone(),
        peer.ping.clone(),
        peer.replies.clone(),
        peer.log.clone(),
    );
    thread::spawn(move || {
        let mut tunn = Tunn::new(
            StaticSecret::from(server_private),
            PublicKey::from(client_public),
            None,
            None,
            1,
            None,
        );
        let mut wire = Wire::default();
        let caps = wire.capabilities();
        let mut config = IfaceConfig::new(HardwareAddress::Ip);
        config.random_seed = 0x5650_4E41;
        let mut iface = Interface::new(config, &mut wire, NetInstant::now());
        iface.update_ip_addrs(|a| {
            let _ = a.push(IpCidr::new(IpAddress::v4(10, 8, 0, 1), 24));
            let _ = a.push(IpCidr::new(IpAddress::v6(0xfd00, 8, 0, 0, 0, 0, 0, 1), 64));
        });
        let mut sockets = SocketSet::new(Vec::new());
        let tcp_new = || {
            tcp::Socket::new(
                tcp::SocketBuffer::new(vec![0; 16384]),
                tcp::SocketBuffer::new(vec![0; 16384]),
            )
        };
        let udp_new = || {
            udp::Socket::new(
                udp::PacketBuffer::new(vec![udp::PacketMetadata::EMPTY; 8], vec![0; 8192]),
                udp::PacketBuffer::new(vec![udp::PacketMetadata::EMPTY; 8], vec![0; 8192]),
            )
        };
        let listeners: Vec<_> = (0..4).map(|_| sockets.add(tcp_new())).collect();
        let mut echo = udp_new();
        let _ = echo.bind(7);
        let echo = sockets.add(echo);
        let mut dns = udp_new();
        let _ = dns.bind(53);
        let dns = sockets.add(dns);
        let icmp = sockets.add(icmp::Socket::new(
            icmp::PacketBuffer::new(vec![icmp::PacketMetadata::EMPTY; 4], vec![0; 1024]),
            icmp::PacketBuffer::new(vec![icmp::PacketMetadata::EMPTY; 4], vec![0; 1024]),
        ));
        let mut guest = None;
        let mut datagram = vec![0u8; 4096];
        let mut buffer = vec![0u8; 65536];
        let mut timers = Instant::now();
        let mut ping_at = Instant::now();
        let mut seq = 0u16;
        while !stop.load(Ordering::Relaxed) {
            // Datagrams from the guest through boringtun.
            if let Ok((n, from)) = socket.recv_from(&mut datagram) {
                guest = Some(from);
                let mut input: &[u8] = &datagram[..n];
                loop {
                    match tunn.decapsulate(Some(from.ip()), input, &mut buffer) {
                        TunnResult::WriteToNetwork(out) => {
                            let _ = socket.send_to(out, from);
                            input = &[];
                        }
                        TunnResult::WriteToTunnelV4(p, _) => {
                            wire.inbound.push_back(p.to_vec());
                            break;
                        }
                        TunnResult::WriteToTunnelV6(p, _) => {
                            wire.inbound.push_back(p.to_vec());
                            note(&log, format!("ipv6 packet of {} bytes", p.len()));
                            break;
                        }
                        TunnResult::Err(e) => {
                            note(&log, format!("boringtun dropped a datagram: {e:?}"));
                            break;
                        }
                        _ => break,
                    }
                }
            }
            iface.poll(NetInstant::now(), &mut wire, &mut sockets);
            // TCP echo on port 7 (four connections at a time).
            for &h in &listeners {
                let s = sockets.get_mut::<tcp::Socket>(h);
                if !s.is_open() {
                    let _ = s.listen(7);
                }
                if s.can_recv() && s.can_send() {
                    let mut b = [0u8; 4096];
                    if let Ok(n) = s.recv_slice(&mut b) {
                        let _ = s.send_slice(&b[..n]);
                    }
                }
                if s.state() == tcp::State::CloseWait && s.send_queue() == 0 {
                    s.close();
                    note(&log, String::from("tcp echo closed"));
                }
            }
            let mut b = [0u8; 2048];
            let s = sockets.get_mut::<udp::Socket>(echo);
            while let Ok((n, meta)) = s.recv_slice(&mut b) {
                let _ = s.send_slice(&b[..n], meta.endpoint);
                note(&log, format!("udp echo {n} from {}", meta.endpoint));
            }
            let s = sockets.get_mut::<udp::Socket>(dns);
            while let Ok((n, meta)) = s.recv_slice(&mut b) {
                if let Some(answer) = dns_answer(&b[..n]) {
                    let _ = s.send_slice(&answer, meta.endpoint);
                    note(&log, format!("dns answered {} bytes", answer.len()));
                }
            }
            // ICMP echo to the guest's tunnel address when asked.
            let s = sockets.get_mut::<icmp::Socket>(icmp);
            if !s.is_open() {
                let _ = s.bind(icmp::Endpoint::Ident(0x2207));
            }
            if ping.load(Ordering::Relaxed)
                && ping_at.elapsed() >= Duration::from_millis(300)
                && s.can_send()
            {
                ping_at = Instant::now();
                seq = seq.wrapping_add(1);
                let repr = Icmpv4Repr::EchoRequest {
                    ident: 0x2207,
                    seq_no: seq,
                    data: b"doors-vpn-ping",
                };
                if let Ok(payload) = s.send(repr.buffer_len(), IpAddress::v4(10, 8, 0, 2)) {
                    repr.emit(&mut Icmpv4Packet::new_unchecked(payload), &caps.checksum);
                }
            }
            while let Ok((payload, from)) = s.recv() {
                if let Ok(packet) = Icmpv4Packet::new_checked(payload)
                    && let Ok(Icmpv4Repr::EchoReply { .. }) =
                        Icmpv4Repr::parse(&packet, &caps.checksum)
                    && from == IpAddress::v4(10, 8, 0, 2)
                {
                    replies.fetch_add(1, Ordering::Relaxed);
                }
            }
            iface.poll(NetInstant::now(), &mut wire, &mut sockets);
            // Packets for the guest through boringtun.
            if let Some(to) = guest {
                for p in wire.outbound.drain(..) {
                    if let TunnResult::WriteToNetwork(out) = tunn.encapsulate(&p, &mut buffer) {
                        let _ = socket.send_to(out, to);
                    }
                }
                if timers.elapsed() >= Duration::from_millis(100) {
                    timers = Instant::now();
                    if let TunnResult::WriteToNetwork(out) = tunn.update_timers(&mut buffer) {
                        let _ = socket.send_to(out, to);
                    }
                }
            } else {
                wire.outbound.clear();
            }
        }
        let (last, tx, rx, _, _) = tunn.stats();
        note(
            &log,
            format!("boringtun: last handshake {last:?} ago, {tx} bytes sent, {rx} bytes received"),
        );
    });
    Ok(peer)
}
/// TCP echo on this host (the guest's 10.0.2.2), for the direct path.
fn host_echo(stop: Arc<AtomicBool>) -> Result<u16, String> {
    let listener = TcpListener::bind("127.0.0.1:0").map_err(|e| e.to_string())?;
    listener.set_nonblocking(true).map_err(|e| e.to_string())?;
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    thread::spawn(move || {
        while !stop.load(Ordering::Relaxed) {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    thread::spawn(move || {
                        let _ = stream.set_nonblocking(false);
                        let _ = stream.set_read_timeout(Some(Duration::from_secs(30)));
                        let mut b = [0u8; 4096];
                        while let Ok(n) = stream.read(&mut b) {
                            if n == 0 || stream.write_all(&b[..n]).is_err() {
                                break;
                            }
                        }
                        let _ = stream.shutdown(Shutdown::Both);
                    });
                }
                Err(_) => thread::sleep(Duration::from_millis(20)),
            }
        }
    });
    Ok(port)
}
/// Waits for `needle` in the serial log after `offset`.
fn serial_after(
    run: &Run,
    record: &Value,
    offset: usize,
    needle: &str,
    seconds: u64,
) -> Result<String, String> {
    let deadline = Instant::now() + Duration::from_secs(seconds);
    loop {
        let text = serial(&run.root, record)?;
        if let Some(line) = text
            .get(offset.min(text.len())..)
            .and_then(|rest| rest.lines().find(|l| l.contains(needle)))
        {
            return Ok(line.trim_end().to_owned());
        }
        if text.contains("KERNEL_PANIC") {
            return Err("guest panicked; inspect serial".into());
        }
        if Instant::now() >= deadline {
            return Err(format!("serial log missing {needle:?}"));
        }
        thread::sleep(Duration::from_millis(100));
    }
}
/// The ECHO lines that write a wg-quick file in the current directory.
fn echo_lines(file: &str, text: &str) -> Vec<String> {
    text.lines()
        .filter(|l| !l.is_empty())
        .enumerate()
        .map(|(i, l)| format!("echo {l} {} {file}", if i == 0 { ">" } else { ">>" }))
        .collect()
}
/// Runs the probe in a new Doorsh tab (Ctrl+T; it finds /system/bin
/// programs), then closes the tab (Ctrl+W) to return to the Command Prompt.
fn probe(
    run: &Run,
    record: &Value,
    c: &mut qmp::Client,
    fixture: &Value,
    line: &str,
    tag: &str,
    steps: &[&str],
) -> Result<Value, String> {
    key(c, &["ctrl", "t"])?;
    thread::sleep(Duration::from_millis(1500));
    let result = run_probe(run, record, c, fixture, line, tag, steps);
    key(c, &["ctrl", "w"])?;
    thread::sleep(Duration::from_millis(1000));
    result
}
/// Types `line` at the Command Prompt, waits until cmd is idle again
/// (`DOOR_P16 CMD IDLE`), and requires `needle` among the lines it printed
/// (`DOOR_P16 CMD TEXT`). Answers those lines.
fn command(
    run: &Run,
    record: &Value,
    c: &mut qmp::Client,
    line: &str,
    needle: &str,
    seconds: u64,
) -> Result<String, String> {
    let before = serial(&run.root, record)?.len();
    type_line(c, line)?;
    serial_after(run, record, before, "DOOR_P16 CMD IDLE", seconds)?;
    let text = serial(&run.root, record)?;
    let printed: Vec<&str> = text
        .get(before.min(text.len())..)
        .unwrap_or("")
        .lines()
        .take_while(|l| !l.contains("DOOR_P16 CMD IDLE"))
        .filter_map(|l| {
            l.split_once("DOOR_P16 CMD TEXT ")
                .map(|(_, t)| t.trim_end())
        })
        .collect();
    let printed = printed.join("\n");
    if !printed.contains(needle) {
        return Err(format!(
            "{line}: expected {needle:?}, cmd printed:\n{printed}"
        ));
    }
    Ok(printed)
}

pub(super) fn verify(run: &mut Run, image: &str) -> Result<(), String> {
    let target = run.generated_directory(&format!("out/builds/{}/vpn-a", run.id))?;
    let fixture = build(run, &target, &VPNPROBE)?;
    let staged = stage(
        run,
        image,
        std::slice::from_ref(&fixture),
        &target,
        "desktop-vpn",
        &["windows/ws2_32.dll"],
    )?;
    let (client, server) = (
        run_key("doors-vpn-client", &run.id),
        run_key("doors-vpn-server", &run.id),
    );
    let wrong_server = door_wireguard::public_key(&run_key("doors-vpn-nobody", &run.id));
    let peer = serve_peer(server, door_wireguard::public_key(&client))?;
    let echo_stop = Arc::new(AtomicBool::new(false));
    let echo_port = host_echo(echo_stop.clone())?;
    let conf = |server_public: [u8; 32]| {
        format!(
            "[Interface]\nPrivateKey = {}\nAddress = 10.8.0.2/24, fd00:8::2/64\nDNS = 10.8.0.1\n[Peer]\nPublicKey = {}\nAllowedIPs = 0.0.0.0/0, ::/0\nEndpoint = 10.0.2.2:{}\nPersistentKeepalive = 25\n",
            door_wireguard::config::encode_key(&client),
            door_wireguard::config::encode_key(&server_public),
            peer.port
        )
    };
    let (office, badkey) = (
        conf(door_wireguard::public_key(&server)),
        conf(wrong_server),
    );
    door_wireguard::Config::parse(&office).map_err(|e| format!("office.conf: {e}"))?;
    let disk = super::win32::fresh_disk(run, "vpn-a")?;
    let mut record = session::launch_desktop_accelerated(run, &staged, &disk, false, RESOLUTION)?;
    let result: Result<Value, String> = (|| {
        let mut c = connection(&record)?;
        thread::sleep(Duration::from_secs(2));
        // Run > cmd: Terminal opens on a Command Prompt tab.
        let before = serial(&run.root, &record)?.len();
        key(&mut c, &["meta_l", "r"])?;
        thread::sleep(Duration::from_millis(400));
        type_line(&mut c, "cmd")?;
        serial_after(run, &record, before, "DOCUMENT_OPEN app=9", 25)?;
        thread::sleep(Duration::from_millis(2500));
        for line in echo_lines("office.conf", &office)
            .iter()
            .chain(&echo_lines("badkey.conf", &badkey))
        {
            command(run, &record, &mut c, line, "", 20)?;
        }
        command(run, &record, &mut c, "rasdial", "No connections", 20)?;
        // Import and connect; door-network reports the first handshake.
        let offset = serial(&run.root, &record)?.len();
        command(
            run,
            &record,
            &mut c,
            "wireguard /installtunnelservice office.conf",
            "",
            20,
        )?;
        let connected = serial_after(
            run,
            &record,
            offset,
            "DOOR_P15 VPN CONNECTED name=office",
            30,
        )?;
        let listed = command(run, &record, &mut c, "rasdial", "Connected to", 20)?;
        let redial = command(
            run,
            &record,
            &mut c,
            "rasdial office",
            "You are already connected to office.",
            20,
        )?;
        let tunnel = probe(
            run,
            &record,
            &mut c,
            &fixture,
            &format!("vpnprobe.exe tunnel {echo_port}"),
            "VPN-A",
            &TUNNEL_STEPS,
        )?;
        // The firewall drops inbound echo requests on a public network, the
        // tunnel's too: the host's pings go unanswered.
        let unanswered = match peer.ping(6) {
            Ok(n) => {
                return Err(format!(
                    "the guest answered {n} pings that no firewall rule allows"
                ));
            }
            Err(e) => e,
        };
        // An administrator allows ICMPv4 in (Win+X > Terminal (Admin),
        // netsh advfirewall); then the pings are answered. Doors runs at most
        // 13 processes, so the Command Prompt's Terminal closes first.
        let before = serial(&run.root, &record)?.len();
        key(&mut c, &["alt", "f4"])?;
        super::win32::wait_after(run, &record, before, "CLIENT_EXIT", 15)?;
        thread::sleep(Duration::from_millis(800));
        let uac = open_admin_terminal(run, &record, &mut c)?;
        let rule_offset = serial(&run.root, &record)?.len();
        let rule = doorsh(
            run,
            &record,
            &mut c,
            "netsh advfirewall firewall add rule name=VpnEcho dir=in action=allow protocol=icmpv4",
            &["Ok."],
        )?;
        let reloaded = serial_after(
            run,
            &record,
            rule_offset,
            "DOOR_P15 FIREWALL RELOAD rules=",
            15,
        )?;
        let pings = peer.ping(15)?;
        let before = serial(&run.root, &record)?.len();
        key(&mut c, &["alt", "f4"])?;
        super::win32::wait_after(run, &record, before, "DOOR_P16 ELEVATED TERMINAL EXIT", 15)?;
        thread::sleep(Duration::from_millis(800));
        let before = serial(&run.root, &record)?.len();
        key(&mut c, &["meta_l", "r"])?;
        thread::sleep(Duration::from_millis(400));
        type_line(&mut c, "cmd")?;
        serial_after(run, &record, before, "DOCUMENT_OPEN app=9", 25)?;
        thread::sleep(Duration::from_millis(2500));
        // Quick settings lists it as connected.
        let tray = serial_after(
            run,
            &record,
            offset,
            "DOOR_P15 TRAY VPN CONNECTED office",
            15,
        )?;
        let frame = scene::capture_settled(run, &mut c, "vpn-a-terminal", &[])?;
        let shot = super::win32::keep(run, &frame, "vpn-a-terminal")?;
        // Settings > Network & internet > VPN lists it as connected.
        let before = serial(&run.root, &record)?.len();
        key(&mut c, &["meta_l", "r"])?;
        thread::sleep(Duration::from_millis(400));
        type_line(&mut c, "ms-settings:network-vpn")?;
        let settings = serial_after(
            run,
            &record,
            before,
            "DOOR_P15 SETTINGS VPN LIST [office: Connected",
            30,
        )?;
        thread::sleep(Duration::from_millis(1500));
        let frame = scene::capture_settled(run, &mut c, "vpn-a-settings", &[])?;
        let settings_shot = super::win32::keep(run, &frame, "vpn-a-settings")?;
        key(&mut c, &["alt", "f4"])?;
        thread::sleep(Duration::from_millis(1500));
        // Hang up: the direct path works again and the tunnel is gone.
        let offset = serial(&run.root, &record)?.len();
        command(
            run,
            &record,
            &mut c,
            "rasdial office /disconnect",
            "Command completed successfully.",
            20,
        )?;
        let killswitch = serial_after(run, &record, offset, "DOOR_P15 VPN KILLSWITCH", 10).ok();
        serial_after(
            run,
            &record,
            offset,
            "DOOR_P15 VPN DISCONNECTED name=office",
            10,
        )?;
        let direct = probe(
            run,
            &record,
            &mut c,
            &fixture,
            &format!("vpnprobe.exe direct {echo_port}"),
            "VPN-A",
            &DIRECT_STEPS,
        )?;
        // A server key nobody holds: no answer, Remote Access error 809.
        command(
            run,
            &record,
            &mut c,
            "wireguard /installtunnelservice badkey.conf",
            "",
            20,
        )?;
        let refused = command(
            run,
            &record,
            &mut c,
            "rasdial badkey",
            "Remote Access error 809",
            45,
        )?;
        let idle = command(run, &record, &mut c, "rasdial", "No connections", 20)?;
        Ok(
            json!({"connected":connected,"tray":tray,"listed":listed.trim(),"redial":redial.trim(),"tunnel":tunnel,
            "host_pings_before_rule":unanswered,"admin_terminal":uac,"echo_rule":rule,"rule_loaded":reloaded,
            "host_pings_answered":pings,"kill_switch_drops":killswitch,"direct":direct,
            "wrong_key":refused.trim(),"after":idle.trim(),"screenshot":shot,"settings":settings,"settings_screenshot":settings_shot}),
        )
    })();
    let details = super::win32::quit_on_error(run, &record, result, "vpn-a-failure");
    let finished = details.as_ref().map_err(String::clone).and_then(|_| {
        let mut c = connection(&record)?;
        super::win32::finish(run, &mut record, &mut c)
    });
    // Boot 2 on the same disk: the desktop shell hands the connections in
    // the user's phonebook back to door.network, and office connects again.
    let restart = finished.and_then(|()| {
        let mut second =
            session::launch_desktop_accelerated(run, &staged, &disk, false, RESOLUTION)?;
        let result: Result<Value, String> = (|| {
            let mut c = connection(&second)?;
            let restored = serial_after(
                run,
                &second,
                0,
                "DOOR_P15 SHELL VPN RESTORED count=2 saved=2",
                90,
            )?;
            thread::sleep(Duration::from_secs(2));
            let before = serial(&run.root, &second)?.len();
            key(&mut c, &["meta_l", "r"])?;
            thread::sleep(Duration::from_millis(400));
            type_line(&mut c, "cmd")?;
            serial_after(run, &second, before, "DOCUMENT_OPEN app=9", 25)?;
            thread::sleep(Duration::from_millis(2500));
            let offset = serial(&run.root, &second)?.len();
            let dialed = command(
                run,
                &second,
                &mut c,
                "rasdial office",
                "Successfully connected to office.",
                45,
            )?;
            let connected = serial_after(
                run,
                &second,
                offset,
                "DOOR_P15 VPN CONNECTED name=office",
                30,
            )?;
            // The restored connection carries traffic like the first.
            let tunnel = probe(
                run,
                &second,
                &mut c,
                &fixture,
                &format!("vpnprobe.exe tunnel {echo_port}"),
                "VPN-A",
                &TUNNEL_STEPS,
            )?;
            command(
                run,
                &second,
                &mut c,
                "rasdial office /disconnect",
                "Command completed successfully.",
                20,
            )?;
            Ok(json!({"restored":restored,"dialed":dialed.trim(),"connected":connected,"tunnel":tunnel,"session":second}))
        })();
        let value = super::win32::quit_on_error(run, &second, result, "vpn-a-restart-failure")?;
        let mut c = connection(&second)?;
        super::win32::finish(run, &mut second, &mut c)?;
        Ok(value)
    });
    peer.stop.store(true, Ordering::Relaxed);
    echo_stop.store(true, Ordering::Relaxed);
    thread::sleep(Duration::from_millis(200));
    let peer_log = peer.log.lock().map(|l| l.clone()).unwrap_or_default();
    let mut details = details?;
    details["after_restart"] = restart?;
    run.check("p15-vpn-a", json!({"fixture":fixture,"image":staged,
        "peer":{"implementation":"boringtun 0.7.1","udp_port":peer.port,"tunnel":"10.8.0.1/24","log":peer_log},
        "client_public_key":door_wireguard::config::encode_key(&door_wireguard::public_key(&client)),
        "host_echo_port":echo_port,"run":details,"session":record,
        "scope":"WireGuard (door-wireguard in door-network) against boringtun over QEMU user networking: handshake, DNS, TCP, UDP and ICMP through the tunnel, the kill-switch, hang-up, and a wrong server key"}));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn the_tunnels_dns_answers_its_one_name() {
        // A query for vpn.doors.test A, id 0x1234, recursion desired.
        let mut q = vec![0x12, 0x34, 0x01, 0x00, 0, 1, 0, 0, 0, 0, 0, 0];
        for label in ["vpn", "doors", "test"] {
            q.push(label.len() as u8);
            q.extend_from_slice(label.as_bytes());
        }
        q.extend_from_slice(&[0, 0, 1, 0, 1]);
        let a = dns_answer(&q).unwrap();
        assert_eq!(&a[..2], &[0x12, 0x34]);
        assert_eq!((a[2], a[3]), (0x85, 0x80));
        assert_eq!(&a[6..8], &[0, 1]);
        assert_eq!(&a[a.len() - 4..], &[10, 8, 0, 1]);
        let mut other = q.clone();
        other[13] = b'x';
        let n = dns_answer(&other).unwrap();
        assert_eq!((n[3] & 0x0F, &n[6..8]), (3, &[0u8, 0][..]));
    }
    #[test]
    fn config_lines_become_echo_commands() {
        let lines = echo_lines("a.conf", "[Interface]\nPrivateKey = x=\n\n[Peer]\n");
        assert_eq!(
            lines,
            [
                "echo [Interface] > a.conf",
                "echo PrivateKey = x= >> a.conf",
                "echo [Peer] >> a.conf"
            ]
        );
    }
}
