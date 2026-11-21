extern crate std;
use super::*;
use alloc::{string::ToString, vec};

/// What Windows 11 printed for tschedprobe's definitions (`raw` on the
/// owner's PC).
const RAW: &str = include_str!("../tests/reference/tschedprobe-raw.txt");

/// The `| `-prefixed lines after the line starting with `marker`, joined as
/// Windows writes them.
fn section(marker: &str) -> String {
    // `marker` is a whole line, or a line's start when it ends with `=`.
    let hit = |l: &&str| {
        if marker.ends_with('=') {
            l.starts_with(marker)
        } else {
            *l == marker
        }
    };
    let mut lines = RAW.lines().skip_while(|l| !hit(l));
    lines.next().unwrap_or_else(|| panic!("no {marker}"));
    lines
        .take_while(|l| l.starts_with("| "))
        .map(|l| &l[2..])
        .collect::<Vec<_>>()
        .join("\r\n")
}
fn s(t: &str) -> Option<String> {
    Some(t.to_string())
}
fn same(ours: &str, windows: &str) {
    for (i, (a, b)) in ours.lines().zip(windows.lines()).enumerate() {
        assert_eq!(a, b, "line {}", i + 1);
    }
    assert_eq!(ours.lines().count(), windows.lines().count());
    assert_eq!(ours, windows);
}
/// tschedprobe's definition step: NewTask, then Queue and PT1H.
fn probe_definition() -> Definition {
    let mut d = Definition::new_task();
    d.settings.instances = 1;
    d.settings.limit = s("PT1H");
    d
}

#[test]
fn new_task_xml_is_windows() {
    same(&xml::write(&probe_definition()), &section("xml-empty"));
}

#[test]
fn probe_task_xml_is_windows() {
    let mut d = probe_definition();
    let mut t = Trigger::new(trigger::TIME);
    t.id = s("Probe");
    t.start = s("2030-01-02T03:04:05");
    t.repetition.interval = s("PT5M");
    d.triggers.push(t);
    let mut a = Action::new(action::EXEC);
    a.path = s(r"C:\Windows\System32\cmd.exe");
    a.arguments = s("/c exit 7");
    d.actions.push(a);
    d.info.author = s("Doors Probe");
    d.info.description = s("A probe task & its <XML>");
    let ours = xml::write(&d);
    same(&ours, &section("definition chars="));
    // Windows' line ends: CRLF between lines, 1572 characters in all.
    assert_eq!(ours.len(), 1572);
    assert_eq!(ours.matches('\r').count(), 43);
    // Read back, written again: the same.
    assert_eq!(xml::read(&ours).map(|d| xml::write(&d)), Ok(ours));
}

/// tschedprobe's fullxml step: every property of every part.
fn full_definition() -> Definition {
    let mut d = Definition::new_task();
    d.info = Registration {
        source: s("Source"),
        date: s("2026-11-09T10:00:00"),
        author: s("Author"),
        version: s("1.0"),
        description: s("Description"),
        documentation: s("Documentation"),
        uri: s(r"\Doors\Uri"),
        security_descriptor: s("D:(A;;FA;;;BA)"),
    };
    let base = |kind: u32, id: Option<&str>| {
        let mut t = Trigger::new(kind);
        t.id = id.map(String::from);
        if id.is_some() && kind != trigger::LOGON {
            t.start = s("2030-01-02T03:04:05");
        }
        t
    };
    let mut t = base(trigger::TIME, Some("T1"));
    t.end = s("2031-01-01T00:00:00");
    t.limit = s("PT2H");
    t.enabled = false;
    t.repetition = Repetition {
        interval: s("PT1H"),
        duration: s("P1D"),
        stop_at_end: true,
    };
    t.random_delay = s("PT30M");
    d.triggers.push(t);
    let mut t = base(trigger::DAILY, Some("T2"));
    t.days_interval = 3;
    t.random_delay = s("PT1M");
    d.triggers.push(t);
    let mut t = base(trigger::WEEKLY, Some("T3"));
    t.days_of_week = 0x22;
    t.weeks_interval = 2;
    d.triggers.push(t);
    let mut t = base(trigger::MONTHLY, Some("T4"));
    t.days_of_month = 0x5;
    t.months = 0x3;
    t.last_day = true;
    d.triggers.push(t);
    let mut t = base(trigger::MONTHLYDOW, Some("T5"));
    t.days_of_week = 0x2;
    t.weeks_of_month = 0x1;
    t.months = 0xFFF;
    t.last_week = true;
    d.triggers.push(t);
    let mut t = base(trigger::LOGON, Some("T6"));
    t.delay = s("PT15S");
    t.user_id = s(r"DOORS\user");
    d.triggers.push(t);
    let mut t = base(trigger::BOOT, None);
    t.delay = s("PT1M");
    d.triggers.push(t);
    d.triggers.push(base(trigger::IDLE, None));
    let mut t = base(trigger::REGISTRATION, None);
    t.delay = s("PT5S");
    d.triggers.push(t);
    let mut t = base(trigger::SESSION_STATE_CHANGE, None);
    t.state_change = 7;
    t.delay = s("PT1S");
    t.user_id = s(r"DOORS\user");
    d.triggers.push(t);
    let mut t = base(trigger::EVENT, None);
    t.subscription = s(
        r#"<QueryList><Query Id="0" Path="System"><Select Path="System">*</Select></Query></QueryList>"#,
    );
    t.delay = s("PT10S");
    d.triggers.push(t);
    d.context = s("Author");
    let mut a = Action::new(action::EXEC);
    a.id = s("A1");
    a.path = s(r"C:\Program Files\Doors\probe.exe");
    a.arguments = s(r#"-a "b c""#);
    a.directory = s(r"C:\Temp");
    d.actions.push(a);
    let mut a = Action::new(action::COM_HANDLER);
    a.class_id = s("{0F87369F-A4E5-4CFC-BD3E-73E6154572DD}");
    a.data = s("data");
    d.actions.push(a);
    let mut a = Action::new(action::SEND_EMAIL);
    a.server = s("smtp.example.com");
    a.subject = s("Subject");
    a.to = s("to@example.com");
    a.from = s("from@example.com");
    a.body = s("Body");
    d.actions.push(a);
    let mut a = Action::new(action::SHOW_MESSAGE);
    a.title = s("Title");
    a.body = s("Message");
    d.actions.push(a);
    let st = &mut d.settings;
    st.demand_start = false;
    st.restart_interval = s("PT5M");
    st.restart_count = 3;
    st.instances = 0;
    st.stop_on_batteries = false;
    st.disallow_on_batteries = false;
    st.hard_terminate = false;
    st.when_available = true;
    st.network_only = true;
    st.limit = s("PT4H");
    st.enabled = false;
    st.delete_after = s("PT1H");
    st.priority = 4;
    st.hidden = true;
    st.idle_only = true;
    st.wake = true;
    st.idle = Idle {
        duration: s("PT5M"),
        wait: s("PT30M"),
        stop_on_end: false,
        restart: true,
    };
    st.network_name = s("Doors Network");
    st.network_id = s("{D0025D01-0000-0000-0000-000000000001}");
    d.principal = Principal {
        id: s("Author"),
        user_id: s(r"DOORS\user"),
        group_id: None,
        display_name: s("Display"),
        logon: 3,
        logon_set: true,
        run_level: 1,
    };
    d.data = s("Some <data>");
    d
}

#[test]
fn full_task_xml_is_windows() {
    let full = section("xml");
    same(&xml::write(&full_definition()), &full);
    // Windows read its own XML back to the same text, with the triggers in
    // order (types 1,2,3,4,5,9,8,6,7,11,0) and four actions.
    let d = xml::read(&full).unwrap();
    let kinds: Vec<u32> = d.triggers.iter().map(|t| t.kind).collect();
    assert_eq!(kinds, vec![1, 2, 3, 4, 5, 9, 8, 6, 7, 11, 0]);
    assert_eq!(d.actions.len(), 4);
    same(&xml::write(&d), &full);
    assert_eq!(d, full_definition());
}

#[test]
fn minimal_xml_takes_schema_defaults() {
    let d = xml::read(&alloc::format!(
        "<Task xmlns=\"{}\"><Actions><Exec><Command>a.exe</Command></Exec></Actions></Task>",
        xml::NAMESPACE
    ))
    .unwrap();
    same(&xml::write(&d), &section("schema unknown="));
}

#[test]
fn malformed_xml_answers_as_windows() {
    // Measured: `<Task>` without the namespace, an element the schema lacks.
    assert_eq!(xml::read("<Task>"), Err(hr::SCHED_E_NAMESPACE));
    let ns = xml::NAMESPACE;
    assert_eq!(
        xml::read(&alloc::format!(
            "<Task version=\"1.2\" xmlns=\"{ns}\"><Bogus/></Task>"
        )),
        Err(hr::SCHED_E_UNEXPECTEDNODE)
    );
    assert_eq!(
        xml::read(&alloc::format!("<Task xmlns=\"{ns}\"><Triggers>")),
        Err(hr::SCHED_E_MALFORMEDXML)
    );
    assert_eq!(
        xml::read(&alloc::format!(
            "<Task xmlns=\"{ns}\"><Settings><Enabled>yes</Enabled></Settings></Task>"
        )),
        Err(hr::SCHED_E_INVALIDVALUE)
    );
    // No action: the definition reads but does not register (measured).
    let d = xml::read(&alloc::format!("<Task xmlns=\"{ns}\"/>")).unwrap();
    assert_eq!(d.validate(), Err(hr::SCHED_E_MISSINGNODE));
}

#[test]
fn boundaries_and_periods() {
    let b = parse_boundary("2030-01-02T03:04:05").unwrap();
    assert_eq!(
        (b.year, b.month, b.day, b.hour, b.minute, b.second, b.offset),
        (2030, 1, 2, 3, 4, 5, None)
    );
    assert_eq!(
        parse_boundary("2030-01-02T03:04:05Z").unwrap().offset,
        Some(0)
    );
    assert_eq!(
        parse_boundary("2030-01-02T03:04:05.123-05:30")
            .unwrap()
            .offset,
        Some(-330)
    );
    for bad in [
        "2030-02-30T00:00:00",
        "2030-01-02 03:04:05",
        "garbage",
        "2030-01-02T24:00:00",
    ] {
        assert!(parse_boundary(bad).is_none(), "{bad}");
    }
    use door_service_protocol::tasks::Schedule;
    assert_eq!(period("PT5M"), Some(Schedule::Minutes(5)));
    assert_eq!(period("PT2H"), Some(Schedule::Hours(2)));
    assert_eq!(period("P3D"), Some(Schedule::Days(3)));
    assert_eq!(period("PT30S"), None);
    assert_eq!(period("P1DT2H"), None);
}

#[test]
fn plans_what_the_scheduler_keeps() {
    use door_service_protocol::tasks::Schedule;
    let mut d = probe_definition();
    let mut t = Trigger::new(trigger::TIME);
    t.start = s("2030-01-02T03:04:05");
    t.repetition.interval = s("PT5M");
    d.triggers.push(t);
    let mut a = Action::new(action::EXEC);
    a.path = s(r"C:\Program Files\p.exe");
    a.arguments = s("exit7");
    d.actions.push(a);
    let p = plan(&d).unwrap();
    assert_eq!(p.schedule, Schedule::Minutes(5));
    assert_eq!(p.command, r#""C:\Program Files\p.exe" exit7"#);
    assert_eq!(p.start.unwrap().year, 2030);
    // A second trigger, an end, a non-Exec action: not kept (deviation).
    let mut two = d.clone();
    two.triggers.push(Trigger::new(trigger::LOGON));
    assert_eq!(plan(&two).err(), Some(hr::NOT_SUPPORTED));
    let mut end = d.clone();
    end.triggers[0].end = s("2031-01-01T00:00:00");
    assert_eq!(plan(&end).err(), Some(hr::NOT_SUPPORTED));
    let mut logon = d.clone();
    logon.triggers = vec![Trigger::new(trigger::LOGON)];
    assert_eq!(plan(&logon).unwrap().schedule, Schedule::OnLogon);
    let mut none = d;
    none.triggers.clear();
    assert!(plan(&none).unwrap().start.is_none());
}
