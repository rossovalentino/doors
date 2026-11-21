use super::*;
use alloc::vec;

const WEB: &str = "v2.33|Action=Allow|Active=TRUE|Dir=In|Protocol=6|Profile=Private|Profile=Public|LPort=8080|App=C:\\Tools\\web.exe|Name=Test web server|Desc=Doors test|EmbedCtxt=Doors tests|";

fn inbound<'a>(port: u16, program: Option<&'a str>) -> Connection<'a> {
    Connection {
        direction: Direction::In,
        protocol: protocol::TCP,
        local_port: port,
        remote_address: Address::V4([10, 0, 2, 2]),
        remote_port: 50000,
        remote_kind: 0,
        icmp: None,
        program,
    }
}

#[test]
fn rules_round_trip_through_the_registry_form() {
    let rule = Rule::parse("{web}", WEB).unwrap();
    assert_eq!(rule.direction, Direction::In);
    assert_eq!(rule.action, Action::Allow);
    assert_eq!(rule.protocol, Some(6));
    assert_eq!(rule.profiles, profile::PRIVATE | profile::PUBLIC);
    assert_eq!(rule.local_ports, vec![(8080, 8080)]);
    assert_eq!(rule.program.as_deref(), Some("C:\\Tools\\web.exe"));
    assert_eq!(Rule::parse("{web}", &rule.text()).unwrap(), rule);
    let ranged = Rule::parse(
        "r",
        "v2.30|Action=Block|Active=FALSE|Dir=Out|RPort=1000-2000|RA4=192.168.0.0/16|RA4=LocalSubnet|Name=Block range|",
    )
    .unwrap();
    assert_eq!(ranged.remote_ports, vec![(1000, 2000)]);
    assert_eq!(ranged.remote_addresses, vec![([192, 168, 0, 0], 16)]);
    assert_eq!(ranged.profiles, profile::ALL);
    assert!(!ranged.enabled);
    for bad in [
        "v1.0|Action=Allow|Dir=In|Name=x|",
        "v2.33|Action=Maybe|Dir=In|Name=x|",
        "v2.33|Action=Allow|Name=x|",
        "v2.33|Action=Allow|Dir=In|",
        "v2.33|Action=Allow|Dir=In|LPort=99999|Name=x|",
        "v2.33|Action=Allow|Dir=In|LPort=20-10|Name=x|",
        "v2.33|Action=Allow|Dir=In|RA4=1.2.3.4/33|Name=x|",
        "v2.33|Action=Allow|Dir=In|novalue|Name=x|",
    ] {
        assert_eq!(Rule::parse("b", bad), Err(Malformed), "{bad}");
    }
}

#[test]
fn inbound_needs_an_allow_rule_and_blocks_win() {
    let mut policy = Policy::default();
    let web = "C:\\TOOLS\\WEB.EXE";
    assert!(!policy.permits(&inbound(8080, Some(web)), profile::PUBLIC));
    policy.rules.push(Rule::parse("{web}", WEB).unwrap());
    assert!(policy.permits(&inbound(8080, Some(web)), profile::PUBLIC));
    // Another program, port or profile is not covered.
    assert!(!policy.permits(&inbound(8080, Some("C:\\other.exe")), profile::PUBLIC));
    assert!(!policy.permits(&inbound(8081, Some(web)), profile::PUBLIC));
    assert!(!policy.permits(&inbound(8080, Some(web)), profile::DOMAIN));
    // Loopback is never filtered; outbound passes by default.
    let mut local = inbound(9999, None);
    local.remote_address = Address::V4([127, 0, 0, 1]);
    assert!(policy.permits(&local, profile::PUBLIC));
    let mut out = inbound(50000, None);
    out.direction = Direction::Out;
    out.remote_port = 443;
    assert!(policy.permits(&out, profile::PUBLIC));
    // A block rule wins over the allow rule.
    policy.rules.push(
        Rule::parse(
            "{b}",
            "v2.33|Action=Block|Active=TRUE|Dir=In|Protocol=6|LPort=8080|Name=No web|",
        )
        .unwrap(),
    );
    assert!(!policy.permits(&inbound(8080, Some(web)), profile::PUBLIC));
    // Shields up blocks even allowed apps; the firewall off allows all.
    policy.rules.pop();
    policy.profiles[2].shields_up = true;
    assert!(!policy.permits(&inbound(8080, Some(web)), profile::PUBLIC));
    policy.profiles[2].enabled = false;
    assert!(policy.permits(&inbound(9, None), profile::PUBLIC));
}

#[test]
fn conditions_the_model_cannot_evaluate_fail_closed() {
    let web = "C:\\TOOLS\\WEB.EXE";
    // An allow rule restricted to a service, an interface or local
    // addresses never matches; the fields survive a round trip.
    for condition in [
        "Svc=Dnscache",
        "IFType=Wireless",
        "LA4=10.0.2.15",
        "LPort=RPC",
        "RA4=Intranet",
        "Security=Authenticate",
    ] {
        let text = format!(
            "v2.33|Action=Allow|Active=TRUE|Dir=In|Protocol=6|LPort=8080|{condition}|Name=Restricted|"
        );
        let rule = Rule::parse("{r}", &text).unwrap();
        assert!(rule.unmodelled, "{condition}");
        assert!(rule.text().contains(condition), "{condition}");
        assert_eq!(Rule::parse("{r}", &rule.text()).unwrap(), rule);
        let policy = Policy {
            rules: vec![rule],
            ..Default::default()
        };
        assert!(
            !policy.permits(&inbound(8080, Some(web)), profile::PUBLIC),
            "{condition}"
        );
    }
    // A block rule with such a condition applies as if it held.
    let mut policy = Policy {
        profiles: [ProfileState {
            inbound_allow: true,
            ..Default::default()
        }; 3],
        ..Default::default()
    };
    assert!(policy.permits(&inbound(8080, Some(web)), profile::PUBLIC));
    policy.rules.push(
        Rule::parse(
            "{b}",
            "v2.33|Action=Block|Active=TRUE|Dir=In|Protocol=6|LPort=8080|Svc=x|Name=B|",
        )
        .unwrap(),
    );
    assert!(!policy.permits(&inbound(8080, Some(web)), profile::PUBLIC));
    // Descriptive keys do not restrict the rule.
    let edge = Rule::parse(
        "{e}",
        "v2.33|Action=Allow|Active=TRUE|Dir=In|Edge=TRUE|Platform=2:6:2|Name=E|",
    )
    .unwrap();
    assert!(!edge.unmodelled);
}

#[test]
fn remote_addresses_ipv6_ranges_and_keywords() {
    let rule = Rule::parse(
        "{a}",
        "v2.33|Action=Allow|Active=TRUE|Dir=In|RA4=10.1.0.0/255.255.0.0|RA4=192.168.1.10-192.168.1.20|RA6=fe80::/64|RA4=LocalSubnet|RA6=LocalSubnet|Name=A|",
    )
    .unwrap();
    assert_eq!(rule.remote_addresses, vec![([10, 1, 0, 0], 16)]);
    assert_eq!(rule.remote_keywords, keyword::LOCAL_SUBNET);
    assert_eq!(Rule::parse("{a}", &rule.text()).unwrap(), rule);
    let policy = Policy {
        rules: vec![rule],
        ..Default::default()
    };
    let at = |address: Address, kind: u8| {
        let mut c = inbound(80, None);
        c.remote_address = address;
        c.remote_kind = kind;
        policy.permits(&c, profile::PUBLIC)
    };
    assert!(at(Address::V4([10, 1, 9, 9]), 0));
    assert!(!at(Address::V4([10, 2, 0, 1]), 0));
    assert!(at(Address::V4([192, 168, 1, 15]), 0));
    assert!(!at(Address::V4([192, 168, 1, 21]), 0));
    let Address::V6(link) = address("fe80::1234").unwrap() else {
        panic!()
    };
    assert!(at(Address::V6(link), 0));
    let Address::V6(global) = address("2001:db8::5").unwrap() else {
        panic!()
    };
    assert!(!at(Address::V6(global), 0));
    assert!(at(Address::V6(global), keyword::LOCAL_SUBNET));
    assert!(at(Address::V4([8, 8, 8, 8]), keyword::LOCAL_SUBNET));
    // IPv4-only conditions do not admit IPv6 peers, and the loopback
    // interface passes in both families.
    let v4_only = Policy {
        rules: vec![
            Rule::parse(
                "{v}",
                "v2.33|Action=Allow|Active=TRUE|Dir=In|RA4=10.0.0.0/8|Name=V|",
            )
            .unwrap(),
        ],
        ..Default::default()
    };
    let mut c = inbound(80, None);
    c.remote_address = Address::V6(global);
    assert!(!v4_only.permits(&c, profile::PUBLIC));
    c.remote_address = address("::1").unwrap();
    assert!(v4_only.permits(&c, profile::PUBLIC));
    c.remote_address = address("::ffff:127.0.0.1").unwrap_or(Address::V6([
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xff, 0xff, 127, 0, 0, 1,
    ]));
    assert!(c.remote_address.is_loopback());
    for bad in [
        "RA6=fe80::/129",
        "RA6=1::2::3",
        "RA4=10.0.0.9-10.0.0.1",
        "RA4=10.0.0.0/255.0.255.0",
        "ICMP4=x:1",
    ] {
        let text = format!("v2.33|Action=Allow|Dir=In|{bad}|Name=x|");
        assert_eq!(Rule::parse("b", &text), Err(Malformed), "{bad}");
    }
}

#[test]
fn icmp_types() {
    let rule = Rule::parse(
        "{i}",
        "v2.33|Action=Allow|Active=TRUE|Dir=In|Protocol=1|ICMP4=8:*|Name=Echo|",
    )
    .unwrap();
    assert_eq!(Rule::parse("{i}", &rule.text()).unwrap(), rule);
    let policy = Policy {
        rules: vec![rule],
        ..Default::default()
    };
    let mut c = inbound(0, None);
    c.protocol = protocol::ICMP;
    c.icmp = Some((8, 0));
    assert!(policy.permits(&c, profile::PUBLIC));
    c.icmp = Some((13, 0));
    assert!(!policy.permits(&c, profile::PUBLIC));
}
