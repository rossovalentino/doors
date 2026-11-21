//! The CONNECT-A test DNS server: an authoritative responder for the
//! `doors.test` zone on this host's loopback (UDP and TCP on one port),
//! reached by the guest at 10.0.2.2. Large TXT answers are truncated over
//! UDP (TC set, no answers) so the client must retry over TCP.
use std::{
    io::{Read, Write},
    net::{TcpListener, UdpSocket},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};

const A: u16 = 1;
const CNAME: u16 = 5;
const SOA: u16 = 6;
const MX: u16 = 15;
const TXT: u16 = 16;
const SRV: u16 = 33;
/// The UDP payload limit without EDNS (RFC 1035 4.2.1).
const UDP_LIMIT: usize = 512;

fn name(text: &str) -> Vec<u8> {
    let mut b = Vec::new();
    for label in text.split('.').filter(|l| !l.is_empty()) {
        b.push(label.len() as u8);
        b.extend_from_slice(label.as_bytes());
    }
    b.push(0);
    b
}
fn record(owner: &str, kind: u16, ttl: u32, data: &[u8]) -> Vec<u8> {
    let mut b = name(owner);
    b.extend_from_slice(&kind.to_be_bytes());
    b.extend_from_slice(&1u16.to_be_bytes());
    b.extend_from_slice(&ttl.to_be_bytes());
    b.extend_from_slice(&(data.len() as u16).to_be_bytes());
    b.extend_from_slice(data);
    b
}
fn soa() -> Vec<u8> {
    let mut d = name("ns.doors.test");
    d.extend(name("hostmaster.doors.test"));
    for v in [2026u32, 3600, 600, 86400, 300] {
        d.extend_from_slice(&v.to_be_bytes());
    }
    record("doors.test", SOA, 300, &d)
}
fn txt(strings: &[String]) -> Vec<u8> {
    let mut d = Vec::new();
    for s in strings {
        d.push(s.len() as u8);
        d.extend_from_slice(s.as_bytes());
    }
    d
}
/// The large TXT record's strings: five of 200 bytes, the last ending in z.
pub fn large_strings() -> Vec<String> {
    (0..5u8)
        .map(|i| {
            let mut s: String = std::iter::repeat_n(char::from(b'a' + i), 199).collect();
            s.push(if i == 4 { 'z' } else { char::from(b'a' + i) });
            s
        })
        .collect()
}
/// Question name (lower case), type, and the question's bytes.
fn question(query: &[u8]) -> Option<(String, u16, Vec<u8>)> {
    let mut at = 12;
    let mut labels = Vec::new();
    loop {
        let n = usize::from(*query.get(at)?);
        if n == 0 {
            at += 1;
            break;
        }
        if n > 63 {
            return None;
        }
        labels.push(String::from_utf8_lossy(query.get(at + 1..at + 1 + n)?).to_ascii_lowercase());
        at += 1 + n;
    }
    let kind = u16::from_be_bytes([*query.get(at)?, *query.get(at + 1)?]);
    Some((labels.join("."), kind, query.get(12..at + 4)?.to_vec()))
}
/// The answer to `query`; `udp` truncates what does not fit 512 bytes.
pub fn answer(query: &[u8], udp: bool) -> Option<Vec<u8>> {
    if query.len() < 12 || query[2] & 0x80 != 0 {
        return None;
    }
    let (qname, kind, question) = question(query)?;
    let host = [10u8, 9, 8, 7];
    let mut answers: Vec<Vec<u8>> = Vec::new();
    let mut authority: Vec<Vec<u8>> = Vec::new();
    let mut rcode = 0u8;
    match (qname.as_str(), kind) {
        ("www.doors.test", A) => {
            answers.push(record(
                "www.doors.test",
                CNAME,
                300,
                &name("host.doors.test"),
            ));
            answers.push(record("host.doors.test", A, 60, &host));
        }
        ("host.doors.test", A) => answers.push(record("host.doors.test", A, 60, &host)),
        ("doors.test", MX) => {
            let mut d = 10u16.to_be_bytes().to_vec();
            d.extend(name("mail.doors.test"));
            answers.push(record("doors.test", MX, 3600, &d));
        }
        ("_probe._tcp.doors.test", SRV) => {
            let mut d = Vec::new();
            for v in [0u16, 5, 8080] {
                d.extend_from_slice(&v.to_be_bytes());
            }
            d.extend(name("svc.doors.test"));
            answers.push(record("_probe._tcp.doors.test", SRV, 600, &d));
        }
        ("medium.doors.test", TXT) => {
            let strings = vec!["m".repeat(150), "n".repeat(150)];
            answers.push(record("medium.doors.test", TXT, 60, &txt(&strings)));
        }
        ("large.doors.test", TXT) => {
            answers.push(record("large.doors.test", TXT, 60, &txt(&large_strings())))
        }
        // Names that exist but hold no record of the type: NODATA.
        (
            "www.doors.test"
            | "host.doors.test"
            | "doors.test"
            | "medium.doors.test"
            | "large.doors.test"
            | "_probe._tcp.doors.test",
            _,
        ) => authority.push(soa()),
        (n, _) if n == "doors.test" || n.ends_with(".doors.test") => {
            rcode = 3;
            authority.push(soa());
        }
        _ => rcode = 5,
    }
    let build = |answers: &[Vec<u8>], authority: &[Vec<u8>], truncated: bool| {
        let mut m = Vec::new();
        m.extend_from_slice(&query[..2]);
        // QR, AA, RD copied, RA; TC when cut.
        m.push(0x84 | (query[2] & 0x01) | if truncated { 0x02 } else { 0 });
        m.push(0x80 | rcode);
        m.extend_from_slice(&1u16.to_be_bytes());
        m.extend_from_slice(&(answers.len() as u16).to_be_bytes());
        m.extend_from_slice(&(authority.len() as u16).to_be_bytes());
        m.extend_from_slice(&0u16.to_be_bytes());
        m.extend_from_slice(&question);
        for r in answers.iter().chain(authority) {
            m.extend_from_slice(r);
        }
        m
    };
    let full = build(&answers, &authority, false);
    if udp && full.len() > UDP_LIMIT {
        return Some(build(&[], &[], true));
    }
    Some(full)
}

pub struct Server {
    pub port: u16,
    stop: Arc<AtomicBool>,
    pub log: Arc<Mutex<Vec<String>>>,
}
impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}
/// Serves until dropped, on one loopback port for both transports.
pub fn serve() -> Result<Server, String> {
    let (tcp, udp) = (0..20)
        .find_map(|_| {
            let tcp = TcpListener::bind("127.0.0.1:0").ok()?;
            let port = tcp.local_addr().ok()?.port();
            let udp = UdpSocket::bind(("127.0.0.1", port)).ok()?;
            Some((tcp, udp))
        })
        .ok_or("no loopback port free for both DNS transports")?;
    let port = udp.local_addr().map_err(|e| e.to_string())?.port();
    let stop = Arc::new(AtomicBool::new(false));
    let log = Arc::new(Mutex::new(Vec::new()));
    udp.set_read_timeout(Some(Duration::from_millis(200)))
        .map_err(|e| e.to_string())?;
    tcp.set_nonblocking(true).map_err(|e| e.to_string())?;
    let (udp_stop, udp_log) = (stop.clone(), log.clone());
    thread::spawn(move || {
        let mut buffer = [0u8; 1500];
        while !udp_stop.load(Ordering::Relaxed) {
            if let Ok((n, peer)) = udp.recv_from(&mut buffer)
                && let Some(reply) = answer(&buffer[..n], true)
            {
                let _ = udp.send_to(&reply, peer);
                if let Ok(mut log) = udp_log.lock() {
                    log.push(format!(
                        "udp {peer} {:?} -> {} bytes",
                        question(&buffer[..n]).map(|q| (q.0, q.1)),
                        reply.len()
                    ));
                }
            }
        }
    });
    let (tcp_stop, tcp_log) = (stop.clone(), log.clone());
    thread::spawn(move || {
        while !tcp_stop.load(Ordering::Relaxed) {
            let Ok((mut stream, peer)) = tcp.accept() else {
                thread::sleep(Duration::from_millis(20));
                continue;
            };
            let log = tcp_log.clone();
            thread::spawn(move || {
                let _ = stream.set_nonblocking(false);
                let _ = stream.set_read_timeout(Some(Duration::from_secs(10)));
                let mut length = [0u8; 2];
                while stream.read_exact(&mut length).is_ok() {
                    let mut query = vec![0u8; usize::from(u16::from_be_bytes(length))];
                    if stream.read_exact(&mut query).is_err() {
                        break;
                    }
                    let Some(reply) = answer(&query, false) else {
                        break;
                    };
                    let mut framed = (reply.len() as u16).to_be_bytes().to_vec();
                    framed.extend_from_slice(&reply);
                    if stream.write_all(&framed).is_err() {
                        break;
                    }
                    if let Ok(mut log) = log.lock() {
                        log.push(format!(
                            "tcp {peer} {:?} -> {} bytes",
                            question(&query).map(|q| (q.0, q.1)),
                            reply.len()
                        ));
                    }
                }
            });
        }
    });
    Ok(Server { port, stop, log })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn query(name_text: &str, kind: u16) -> Vec<u8> {
        let mut q = vec![0x12, 0x34, 0x01, 0x00, 0, 1, 0, 0, 0, 0, 0, 0];
        q.extend(name(name_text));
        q.extend_from_slice(&kind.to_be_bytes());
        q.extend_from_slice(&1u16.to_be_bytes());
        q
    }
    #[test]
    fn zone_answers_and_truncation() {
        let a = answer(&query("WWW.doors.test", A), true).unwrap();
        assert_eq!(&a[..2], &[0x12, 0x34]);
        assert_eq!(a[3] & 0xf, 0);
        assert_eq!(u16::from_be_bytes([a[6], a[7]]), 2);
        let large = answer(&query("large.doors.test", TXT), true).unwrap();
        assert!(
            large.len() <= UDP_LIMIT
                && large[2] & 0x02 != 0
                && u16::from_be_bytes([large[6], large[7]]) == 0
        );
        let whole = answer(&query("large.doors.test", TXT), false).unwrap();
        assert!(whole.len() > 1000 && whole[2] & 0x02 == 0);
        let medium = answer(&query("medium.doors.test", TXT), true).unwrap();
        assert!(medium.len() > 192 && medium.len() <= UDP_LIMIT && medium[2] & 0x02 == 0);
        let missing = answer(&query("missing.doors.test", A), true).unwrap();
        assert_eq!(
            (
                missing[3] & 0xf,
                u16::from_be_bytes([missing[8], missing[9]])
            ),
            (3, 1)
        );
        let nodata = answer(&query("host.doors.test", 28), true).unwrap();
        assert_eq!(
            (
                nodata[3] & 0xf,
                u16::from_be_bytes([nodata[6], nodata[7]]),
                u16::from_be_bytes([nodata[8], nodata[9]])
            ),
            (0, 0, 1)
        );
        assert_eq!(answer(&query("example.com", A), true).unwrap()[3] & 0xf, 5);
        assert_eq!(large_strings()[4].as_bytes()[199], b'z');
    }
}
