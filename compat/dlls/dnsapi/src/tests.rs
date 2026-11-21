use super::*;
use alloc::vec;

/// DnsValidateName_W results for formats 0..=6 measured on this PC's
/// Windows 11 26H2 (tests/fixtures note: read-only host program).
const MEASURED: &[(&str, [u32; 7])] = &[
    ("example.com", [0, 123, 0, 123, 123, 123, 0]),
    ("example.com.", [0, 123, 0, 123, 123, 123, 0]),
    ("my_host", [9556, 9556, 9556, 9556, 123, 123, 0]),
    ("_sip._tcp.example.com", [9556, 123, 9556, 123, 123, 0, 0]),
    ("-bad", [0, 0, 0, 0, 123, 123, 0]),
    ("good-", [0, 0, 0, 0, 123, 123, 0]),
    ("a..b", [123, 123, 123, 123, 123, 123, 0]),
    ("", [123, 123, 123, 123, 123, 123, 0]),
    ("123", [0, 0, 0, 0, 123, 123, 0]),
    ("123.example.com", [0, 123, 0, 123, 123, 123, 0]),
    ("host.123", [0, 123, 0, 123, 123, 123, 9561]),
    ("*.example.com", [9560, 123, 9560, 123, 0, 123, 0]),
    ("a*b.com", [9560, 123, 9560, 123, 123, 123, 0]),
    ("ex ample.com", [9560, 123, 9560, 123, 123, 123, 0]),
    ("ex@mple.com", [9560, 123, 9560, 123, 123, 123, 0]),
    ("caf\u{e9}.com", [0, 123, 0, 123, 123, 123, 0]),
    ("a/b", [9560, 9560, 9560, 9560, 123, 123, 0]),
    ("x", [0, 0, 0, 0, 123, 123, 0]),
    ("a_b.c", [9556, 123, 9556, 123, 123, 123, 0]),
    (
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa.com",
        [123, 123, 123, 123, 123, 123, 0],
    ),
    ("host!", [9560, 9560, 9560, 9560, 123, 123, 0]),
    ("ho$t", [9560, 9560, 9560, 9560, 123, 123, 0]),
    ("ho:t", [9560, 9560, 9560, 9560, 123, 123, 0]),
    ("ho+t", [9560, 9560, 9560, 9560, 123, 123, 0]),
    ("ho~t", [9560, 9560, 9560, 9560, 123, 123, 0]),
    ("ho\"t", [9560, 9560, 9560, 9560, 123, 123, 0]),
];

#[test]
fn validation_matches_windows() {
    for (name, expected) in MEASURED {
        for (format, want) in expected.iter().enumerate() {
            assert_eq!(
                validate_name(name, format as u32),
                *want,
                "{name:?} format {format}"
            );
        }
    }
    assert_eq!(validate_name("a.com", 7), status::ERROR_INVALID_PARAMETER);
    assert!(
        names_equal("Example.COM", "example.com.")
            && names_equal("a.com.", "A.COM")
            && !names_equal("a.com", "b.com")
    );
}

#[test]
fn queries_encode_and_reject_bad_names() {
    let q = encode_query(0x1234, "www.example.com", kind::AAAA).unwrap();
    assert_eq!(&q[..12], &[0x12, 0x34, 0x01, 0x00, 0, 1, 0, 0, 0, 0, 0, 0]);
    assert_eq!(&q[12..29], b"\x03www\x07example\x03com\x00");
    assert_eq!(&q[29..], &[0, 28, 0, 1]);
    assert_eq!(
        encode_query(1, "example.com.", kind::A).unwrap().len(),
        12 + 13 + 4
    );
    assert_eq!(
        encode_query(1, "a..b", kind::A),
        Err(status::ERROR_INVALID_NAME)
    );
    assert_eq!(
        encode_query(1, &"a".repeat(64), kind::A),
        Err(status::ERROR_INVALID_NAME)
    );
    assert_eq!(reverse_name([10, 0, 2, 15]), "15.2.0.10.in-addr.arpa");
    assert_eq!(ipv4_literal("10.0.2.3"), Some([10, 0, 2, 3]));
    assert!(
        ipv4_literal("10.0.2").is_none()
            && ipv4_literal("1.2.3.4.5").is_none()
            && ipv4_literal("256.1.1.1").is_none()
    );
}

/// A response with compression pointers in every section.
fn response() -> Vec<u8> {
    let mut m = vec![0xab, 0xcd, 0x81, 0x80, 0, 1, 0, 6, 0, 1, 0, 1];
    // Question: www.example.com A (name at 12, "example.com" at 16).
    m.extend_from_slice(b"\x03www\x07example\x03com\x00\x00\x01\x00\x01");
    let rr = |m: &mut Vec<u8>, name: &[u8], kind: u16, ttl: u32, data: &[u8]| {
        m.extend_from_slice(name);
        m.extend_from_slice(&kind.to_be_bytes());
        m.extend_from_slice(&1u16.to_be_bytes());
        m.extend_from_slice(&ttl.to_be_bytes());
        m.extend_from_slice(&(data.len() as u16).to_be_bytes());
        m.extend_from_slice(data);
    };
    rr(&mut m, &[0xc0, 12], kind::CNAME, 300, b"\x04edge\xc0\x10");
    rr(
        &mut m,
        b"\x04edge\xc0\x10",
        kind::A,
        60,
        &[93, 184, 216, 34],
    );
    rr(
        &mut m,
        &[0xc0, 16],
        kind::MX,
        3600,
        b"\x00\x0a\x04mail\xc0\x10",
    );
    rr(&mut m, &[0xc0, 16], kind::TXT, 10, b"\x05hello\x00\x03v=1");
    rr(
        &mut m,
        b"\x04_sip\x04_tcp\xc0\x10",
        kind::SRV,
        20,
        b"\x00\x01\x00\x02\x13\xc4\x03sip\xc0\x10",
    );
    rr(
        &mut m,
        &[0xc0, 16],
        kind::AAAA,
        30,
        &[0x20, 0x01, 0x0d, 0xb8, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1],
    );
    rr(&mut m, &[0xc0, 16], kind::SOA, 900, b"\x02ns\xc0\x10\x0ahostmaster\xc0\x10\x00\x00\x00\x07\x00\x00\x0e\x10\x00\x00\x02\x58\x00\x09\x3a\x80\x00\x00\x01\x2c");
    // An OPT pseudo-record in the additional section is skipped.
    m.extend_from_slice(&[0, 0, 41, 0x10, 0, 0, 0, 0, 0, 0, 0]);
    m
}

#[test]
fn responses_decode_every_section_and_type() {
    let r = parse_response(&response()).unwrap();
    assert_eq!((r.id, r.rcode, r.truncated), (0xabcd, 0, false));
    let kinds: Vec<(u16, Section)> = r.records.iter().map(|x| (x.kind, x.section)).collect();
    assert_eq!(
        kinds,
        [
            (kind::CNAME, Section::Answer),
            (kind::A, Section::Answer),
            (kind::MX, Section::Answer),
            (kind::TXT, Section::Answer),
            (kind::SRV, Section::Answer),
            (kind::AAAA, Section::Answer),
            (kind::SOA, Section::Authority),
        ]
    );
    assert_eq!(r.records[0].name, "www.example.com");
    assert_eq!(r.records[0].data, Data::Name("edge.example.com".into()));
    assert_eq!(
        (r.records[1].name.as_str(), r.records[1].ttl),
        ("edge.example.com", 60)
    );
    assert_eq!(r.records[1].data, Data::A([93, 184, 216, 34]));
    assert_eq!(
        r.records[2].data,
        Data::Mx {
            preference: 10,
            exchange: "mail.example.com".into()
        }
    );
    assert_eq!(
        r.records[3].data,
        Data::Txt(vec!["hello".into(), "".into(), "v=1".into()])
    );
    assert_eq!(r.records[4].name, "_sip._tcp.example.com");
    assert_eq!(
        r.records[4].data,
        Data::Srv {
            priority: 1,
            weight: 2,
            port: 5060,
            target: "sip.example.com".into()
        }
    );
    assert_eq!(
        r.records[6].data,
        Data::Soa {
            primary: "ns.example.com".into(),
            admin: "hostmaster.example.com".into(),
            serial: 7,
            refresh: 3600,
            retry: 600,
            expire: 604800,
            minimum: 300
        }
    );
    assert_eq!(outcome(&r, kind::A), status::SUCCESS);
    assert_eq!(outcome(&r, kind::PTR), status::INFO_NO_RECORDS);
}

#[test]
fn malformed_responses_and_rcodes() {
    let good = response();
    // Truncation anywhere is a bad packet, never a panic.
    for cut in 0..good.len() - 11 {
        let _ = parse_response(&good[..cut]);
    }
    assert_eq!(parse_response(&good[..40]), Err(status::BAD_PACKET));
    // A query (QR clear) is not a response.
    let mut query = good.clone();
    query[2] &= 0x7f;
    assert_eq!(parse_response(&query), Err(status::BAD_PACKET));
    // A pointer loop (to itself or forward) is refused.
    let mut looped = good.clone();
    looped[12] = 0xc0;
    looped[13] = 12;
    assert_eq!(parse_response(&looped), Err(status::BAD_PACKET));
    for (rcode, want) in [(1, 9001), (2, 9002), (3, 9003), (4, 9004), (5, 9005)] {
        let mut m = good.clone();
        m[3] = 0x80 | rcode;
        assert_eq!(outcome(&parse_response(&m).unwrap(), kind::A), want);
    }
    let mut truncated = good;
    truncated[2] |= 0x02;
    assert!(parse_response(&truncated).unwrap().truncated);
}

#[test]
fn replies_must_answer_the_question() {
    let query = encode_query(0xabcd, "www.example.com", kind::A).unwrap();
    let reply = response();
    assert!(answers(&query, &reply));
    // 0x20 case randomization is not a different question.
    let mixed = encode_query(0xabcd, "WWW.Example.COM", kind::A).unwrap();
    assert!(answers(&mixed, &reply));
    // Another ID, name or type is not this query's answer.
    assert!(!answers(
        &encode_query(0xabce, "www.example.com", kind::A).unwrap(),
        &reply
    ));
    assert!(!answers(
        &encode_query(0xabcd, "www.example.org", kind::A).unwrap(),
        &reply
    ));
    assert!(!answers(
        &encode_query(0xabcd, "www.example.com", kind::AAAA).unwrap(),
        &reply
    ));
    assert!(!answers(&query, &reply[..20]));
    // Types 65 (HTTPS, 'A') and 97 ('a') differ only in the 0x20 bit: the
    // type is a number, never case-folded.
    let https = encode_query(0xabcd, "www.example.com", 65).unwrap();
    let mut other = https.clone();
    let at = other.len() - 3;
    other[at] = 97;
    assert!(answers(&https, &https));
    assert!(!answers(&https, &other));
}

/// Hostile replies: mutations of a real answer (compression pointers, counts,
/// lengths, truncation) never panic the parser or the question check, and
/// a reply whose header or question changed is never taken as this query's
/// answer.
#[test]
fn hostile_replies_never_panic_or_answer_another_question() {
    let query = encode_query(0xabcd, "www.example.com", kind::A).unwrap();
    let seed = response();
    let mut state = 0x2545_f491_4f6c_dd1du64;
    let mut next = move || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    for _ in 0..60_000 {
        let mut m = seed.clone();
        for _ in 0..1 + next() % 5 {
            let r = next();
            match r % 5 {
                0 | 1 if !m.is_empty() => {
                    let at = (r >> 8) as usize % m.len();
                    m[at] = (r >> 32) as u8;
                }
                // A compression pointer to anywhere.
                2 if m.len() > 14 => {
                    let at = 12 + (r >> 8) as usize % (m.len() - 13);
                    m[at] = 0xc0 | (r >> 40) as u8 & 0x3f;
                    m[at + 1] = (r >> 48) as u8;
                }
                3 => m.truncate((r >> 8) as usize % (m.len() + 1)),
                _ => m.extend_from_slice(&(r >> 8).to_le_bytes()[..(r % 8) as usize]),
            }
        }
        let _ = parse_response(&m);
        // An independent oracle: the ID, one question, the name octet by
        // octet with only A-Z/a-z folded, then QTYPE and QCLASS exact.
        let fold = |b: u8| if b.is_ascii_uppercase() { b + 32 } else { b };
        let tail = query.len() - 4;
        let expected = m.len() >= query.len()
            && m[..2] == query[..2]
            && m[4..6] == [0, 1]
            && (12..tail).all(|i| fold(m[i]) == fold(query[i]))
            && m[tail..query.len()] == query[tail..];
        assert_eq!(answers(&query, &m), expected, "{m:02x?}");
    }
}
