//! Task XML (the Task Scheduler schema, `http://schemas.microsoft.com/
//! windows/2004/02/mit/task`): a definition written in the element order,
//! indentation and line ends Windows 11 uses (two spaces, CRLF between
//! lines, none after the last), and read back from any XML of that
//! schema the model holds.
use super::*;

pub const NAMESPACE: &str = "http://schemas.microsoft.com/windows/2004/02/mit/task";

// ---- writing ----------------------------------------------------------------------------
struct Writer {
    out: String,
    depth: usize,
}
fn escape(text: &str, attribute: bool, out: &mut String) {
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' if attribute => out.push_str("&quot;"),
            c => out.push(c),
        }
    }
}
impl Writer {
    fn line(&mut self) {
        if !self.out.is_empty() {
            self.out.push_str("\r\n");
        }
        for _ in 0..self.depth {
            self.out.push_str("  ");
        }
    }
    fn tag(&mut self, name: &str, attrs: &[(&str, Option<&str>)]) {
        self.out.push('<');
        self.out.push_str(name);
        for (k, v) in attrs {
            if let Some(v) = v {
                self.out.push(' ');
                self.out.push_str(k);
                self.out.push_str("=\"");
                escape(v, true, &mut self.out);
                self.out.push('"');
            }
        }
    }
    fn open(&mut self, name: &str, attrs: &[(&str, Option<&str>)]) {
        self.line();
        self.tag(name, attrs);
        self.out.push('>');
        self.depth += 1;
    }
    fn close(&mut self, name: &str) {
        self.depth -= 1;
        self.line();
        self.out.push_str("</");
        self.out.push_str(name);
        self.out.push('>');
    }
    fn empty(&mut self, name: &str, attrs: &[(&str, Option<&str>)]) {
        self.line();
        self.tag(name, attrs);
        self.out.push_str(" />");
    }
    fn leaf(&mut self, name: &str, text: &str) {
        self.line();
        self.out.push('<');
        self.out.push_str(name);
        self.out.push('>');
        escape(text, false, &mut self.out);
        self.out.push_str("</");
        self.out.push_str(name);
        self.out.push('>');
    }
    fn maybe(&mut self, name: &str, text: &Option<String>) {
        if let Some(t) = text {
            self.leaf(name, t);
        }
    }
    fn flag(&mut self, name: &str, value: bool) {
        self.leaf(name, if value { "true" } else { "false" });
    }
}
pub const DAYS: [&str; 7] = [
    "Sunday",
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
];
pub const MONTHS: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];
pub const INSTANCES: [&str; 4] = ["Parallel", "Queue", "IgnoreNew", "StopExisting"];
pub const LOGON_TYPES: [&str; 7] = [
    "None",
    "Password",
    "S4U",
    "InteractiveToken",
    "Group",
    "ServiceAccount",
    "InteractiveTokenOrPassword",
];
pub const STATE_CHANGES: [(u32, &str); 6] = [
    (1, "ConsoleConnect"),
    (2, "ConsoleDisconnect"),
    (3, "RemoteConnect"),
    (4, "RemoteDisconnect"),
    (7, "SessionLock"),
    (8, "SessionUnlock"),
];
fn bit_names(w: &mut Writer, outer: &str, bits: u32, names: &[&str]) {
    if bits == 0 {
        w.empty(outer, &[]);
        return;
    }
    w.open(outer, &[]);
    for (i, n) in names.iter().enumerate() {
        if bits & (1 << i) != 0 {
            w.empty(n, &[]);
        }
    }
    w.close(outer);
}
fn trigger_name(kind: u32) -> &'static str {
    match kind {
        trigger::TIME => "TimeTrigger",
        trigger::DAILY | trigger::WEEKLY | trigger::MONTHLY | trigger::MONTHLYDOW => {
            "CalendarTrigger"
        }
        trigger::IDLE => "IdleTrigger",
        trigger::REGISTRATION => "RegistrationTrigger",
        trigger::BOOT => "BootTrigger",
        trigger::LOGON => "LogonTrigger",
        trigger::SESSION_STATE_CHANGE => "SessionStateChangeTrigger",
        _ => "EventTrigger",
    }
}
fn write_trigger(w: &mut Writer, t: &Trigger) {
    let name = trigger_name(t.kind);
    w.open(name, &[("id", t.id.as_deref())]);
    if let Some(interval) = &t.repetition.interval {
        w.open("Repetition", &[]);
        w.leaf("Interval", interval);
        w.maybe("Duration", &t.repetition.duration);
        w.flag("StopAtDurationEnd", t.repetition.stop_at_end);
        w.close("Repetition");
    }
    w.maybe("StartBoundary", &t.start);
    w.maybe("EndBoundary", &t.end);
    w.maybe("ExecutionTimeLimit", &t.limit);
    w.flag("Enabled", t.enabled);
    match t.kind {
        trigger::TIME => w.maybe("RandomDelay", &t.random_delay),
        trigger::DAILY => {
            w.maybe("RandomDelay", &t.random_delay);
            w.open("ScheduleByDay", &[]);
            w.leaf("DaysInterval", &alloc::format!("{}", t.days_interval));
            w.close("ScheduleByDay");
        }
        trigger::WEEKLY => {
            w.maybe("RandomDelay", &t.random_delay);
            w.open("ScheduleByWeek", &[]);
            bit_names(w, "DaysOfWeek", t.days_of_week as u16 as u32, &DAYS);
            w.leaf("WeeksInterval", &alloc::format!("{}", t.weeks_interval));
            w.close("ScheduleByWeek");
        }
        trigger::MONTHLY => {
            w.maybe("RandomDelay", &t.random_delay);
            w.open("ScheduleByMonth", &[]);
            w.open("DaysOfMonth", &[]);
            for day in 0..31 {
                if t.days_of_month as u32 & (1 << day) != 0 {
                    w.leaf("Day", &alloc::format!("{}", day + 1));
                }
            }
            if t.last_day {
                w.leaf("Day", "Last");
            }
            w.close("DaysOfMonth");
            bit_names(w, "Months", t.months as u16 as u32, &MONTHS);
            w.close("ScheduleByMonth");
        }
        trigger::MONTHLYDOW => {
            w.maybe("RandomDelay", &t.random_delay);
            w.open("ScheduleByMonthDayOfWeek", &[]);
            w.open("Weeks", &[]);
            for week in 0..4 {
                if t.weeks_of_month as u16 & (1 << week) != 0 {
                    w.leaf("Week", &alloc::format!("{}", week + 1));
                }
            }
            if t.last_week {
                w.leaf("Week", "Last");
            }
            w.close("Weeks");
            bit_names(w, "DaysOfWeek", t.days_of_week as u16 as u32, &DAYS);
            bit_names(w, "Months", t.months as u16 as u32, &MONTHS);
            w.close("ScheduleByMonthDayOfWeek");
        }
        trigger::LOGON => {
            w.maybe("UserId", &t.user_id);
            w.maybe("Delay", &t.delay);
        }
        trigger::BOOT | trigger::REGISTRATION => w.maybe("Delay", &t.delay),
        trigger::SESSION_STATE_CHANGE => {
            if let Some((_, n)) = STATE_CHANGES.iter().find(|(v, _)| *v == t.state_change) {
                w.leaf("StateChange", n);
            }
            w.maybe("UserId", &t.user_id);
            w.maybe("Delay", &t.delay);
        }
        trigger::EVENT => {
            w.maybe("Subscription", &t.subscription);
            w.maybe("Delay", &t.delay);
            if !t.value_queries.is_empty() {
                w.open("ValueQueries", &[]);
                for (name, value) in &t.value_queries {
                    w.line();
                    w.tag("Value", &[("name", Some(name))]);
                    w.out.push('>');
                    escape(value, false, &mut w.out);
                    w.out.push_str("</Value>");
                }
                w.close("ValueQueries");
            }
        }
        _ => {}
    }
    w.close(name);
}
fn write_action(w: &mut Writer, a: &Action) {
    let id = [("id", a.id.as_deref())];
    match a.kind {
        action::EXEC => {
            w.open("Exec", &id);
            w.maybe("Command", &a.path);
            w.maybe("Arguments", &a.arguments);
            w.maybe("WorkingDirectory", &a.directory);
            w.close("Exec");
        }
        action::COM_HANDLER => {
            w.open("ComHandler", &id);
            w.maybe("ClassId", &a.class_id);
            w.maybe("Data", &a.data);
            w.close("ComHandler");
        }
        action::SEND_EMAIL => {
            w.open("SendEmail", &id);
            w.maybe("Server", &a.server);
            w.maybe("Subject", &a.subject);
            w.maybe("To", &a.to);
            w.maybe("Cc", &a.cc);
            w.maybe("Bcc", &a.bcc);
            w.maybe("ReplyTo", &a.reply_to);
            w.maybe("From", &a.from);
            w.maybe("Body", &a.body);
            if a.header_fields.is_empty() {
                w.empty("HeaderFields", &[]);
            } else {
                w.open("HeaderFields", &[]);
                for (name, value) in &a.header_fields {
                    w.open("HeaderField", &[]);
                    w.leaf("Name", name);
                    w.leaf("Value", value);
                    w.close("HeaderField");
                }
                w.close("HeaderFields");
            }
            w.empty("Attachments", &[]);
            w.close("SendEmail");
        }
        _ => {
            w.open("ShowMessage", &id);
            w.maybe("Title", &a.title);
            w.maybe("Body", &a.body);
            w.close("ShowMessage");
        }
    }
}
/// The definition's XML (ITaskDefinition::XmlText).
pub fn write(d: &Definition) -> String {
    let mut w = Writer {
        out: String::from("<?xml version=\"1.0\" encoding=\"UTF-16\"?>"),
        depth: 0,
    };
    let version = alloc::format!("1.{}", d.settings.compatibility.min(9));
    w.open(
        "Task",
        &[("version", Some(&version)), ("xmlns", Some(NAMESPACE))],
    );
    let r = &d.info;
    let parts = [
        ("Source", &r.source),
        ("Date", &r.date),
        ("Author", &r.author),
        ("Version", &r.version),
        ("Description", &r.description),
        ("Documentation", &r.documentation),
        ("URI", &r.uri),
        ("SecurityDescriptor", &r.security_descriptor),
    ];
    if parts.iter().all(|(_, v)| v.is_none()) {
        w.empty("RegistrationInfo", &[]);
    } else {
        w.open("RegistrationInfo", &[]);
        for (n, v) in parts {
            w.maybe(n, v);
        }
        w.close("RegistrationInfo");
    }
    if d.triggers.is_empty() {
        w.empty("Triggers", &[]);
    } else {
        w.open("Triggers", &[]);
        for t in &d.triggers {
            write_trigger(&mut w, t);
        }
        w.close("Triggers");
    }
    let p = &d.principal;
    if p.id.is_some()
        || p.user_id.is_some()
        || p.group_id.is_some()
        || p.display_name.is_some()
        || p.logon_set
        || p.run_level != 0
    {
        w.open("Principals", &[]);
        w.open("Principal", &[("id", p.id.as_deref())]);
        w.maybe("UserId", &p.user_id);
        w.maybe("GroupId", &p.group_id);
        if p.logon_set {
            w.leaf(
                "LogonType",
                LOGON_TYPES
                    .get(p.logon as usize)
                    .copied()
                    .unwrap_or("InteractiveToken"),
            );
        }
        w.maybe("DisplayName", &p.display_name);
        if p.run_level != 0 {
            w.leaf("RunLevel", "HighestAvailable");
        }
        w.close("Principal");
        w.close("Principals");
    }
    let s = &d.settings;
    w.open("Settings", &[]);
    w.leaf(
        "MultipleInstancesPolicy",
        INSTANCES
            .get(s.instances as usize)
            .copied()
            .unwrap_or("IgnoreNew"),
    );
    w.flag("DisallowStartIfOnBatteries", s.disallow_on_batteries);
    w.flag("StopIfGoingOnBatteries", s.stop_on_batteries);
    w.flag("AllowHardTerminate", s.hard_terminate);
    w.flag("StartWhenAvailable", s.when_available);
    w.flag("RunOnlyIfNetworkAvailable", s.network_only);
    if s.network_name.is_some() || s.network_id.is_some() {
        w.open("NetworkSettings", &[]);
        w.maybe("Name", &s.network_name);
        w.maybe("Id", &s.network_id);
        w.close("NetworkSettings");
    }
    w.open("IdleSettings", &[]);
    w.maybe("Duration", &s.idle.duration);
    w.maybe("WaitTimeout", &s.idle.wait);
    w.flag("StopOnIdleEnd", s.idle.stop_on_end);
    w.flag("RestartOnIdle", s.idle.restart);
    w.close("IdleSettings");
    w.flag("AllowStartOnDemand", s.demand_start);
    w.flag("Enabled", s.enabled);
    w.flag("Hidden", s.hidden);
    w.flag("RunOnlyIfIdle", s.idle_only);
    if s.remote_app {
        w.flag("DisallowStartOnRemoteAppSession", true);
    }
    if s.unified {
        w.flag("UseUnifiedSchedulingEngine", true);
    }
    w.flag("WakeToRun", s.wake);
    w.maybe("ExecutionTimeLimit", &s.limit);
    w.maybe("DeleteExpiredTaskAfter", &s.delete_after);
    w.leaf("Priority", &alloc::format!("{}", s.priority));
    if let Some(interval) = &s.restart_interval {
        w.open("RestartOnFailure", &[]);
        w.leaf("Interval", interval);
        w.leaf("Count", &alloc::format!("{}", s.restart_count));
        w.close("RestartOnFailure");
    }
    if s.volatile {
        w.flag("Volatile", true);
    }
    w.close("Settings");
    if let Some(data) = &d.data {
        w.line();
        w.out.push_str("<Data><![CDATA[");
        w.out.push_str(data);
        w.out.push_str("]]></Data>");
    }
    if d.actions.is_empty() {
        w.empty("Actions", &[("Context", d.context.as_deref())]);
    } else {
        w.open("Actions", &[("Context", d.context.as_deref())]);
        for a in &d.actions {
            write_action(&mut w, a);
        }
        w.close("Actions");
    }
    w.close("Task");
    w.out
}

// ---- reading -----------------------------------------------------------------------------
#[derive(Debug, Default)]
pub struct Node {
    pub name: String,
    pub attrs: Vec<(String, String)>,
    pub children: Vec<Node>,
    pub text: String,
}
impl Node {
    fn attr(&self, name: &str) -> Option<&str> {
        self.attrs
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    }
}
struct Reader<'a> {
    s: &'a str,
    at: usize,
}
const MALFORMED: u32 = hr::SCHED_E_MALFORMEDXML;
impl Reader<'_> {
    fn rest(&self) -> &str {
        &self.s[self.at..]
    }
    fn skip_space(&mut self) {
        while self.rest().starts_with([' ', '\t', '\r', '\n']) {
            self.at += 1;
        }
    }
    /// Comments and processing instructions between elements.
    fn skip_misc(&mut self) -> Result<(), u32> {
        loop {
            self.skip_space();
            if self.rest().starts_with("<?") {
                let end = self.rest().find("?>").ok_or(MALFORMED)?;
                self.at += end + 2;
            } else if self.rest().starts_with("<!--") {
                let end = self.rest().find("-->").ok_or(MALFORMED)?;
                self.at += end + 3;
            } else {
                return Ok(());
            }
        }
    }
    fn name(&mut self) -> Result<String, u32> {
        let n = self
            .rest()
            .find(|c: char| c.is_whitespace() || matches!(c, '>' | '/' | '='))
            .ok_or(MALFORMED)?;
        if n == 0 {
            return Err(MALFORMED);
        }
        let name = String::from(&self.rest()[..n]);
        self.at += n;
        Ok(name)
    }
    /// `<name attrs` up to `>` or `/>`: the node and whether it is empty.
    fn start(&mut self) -> Result<(Node, bool), u32> {
        if !self.rest().starts_with('<') {
            return Err(MALFORMED);
        }
        self.at += 1;
        let mut node = Node {
            name: self.name()?,
            ..Node::default()
        };
        loop {
            self.skip_space();
            if self.rest().starts_with("/>") {
                self.at += 2;
                return Ok((node, true));
            }
            if self.rest().starts_with('>') {
                self.at += 1;
                return Ok((node, false));
            }
            let key = self.name()?;
            self.skip_space();
            if !self.rest().starts_with('=') {
                return Err(MALFORMED);
            }
            self.at += 1;
            self.skip_space();
            let quote = self.rest().chars().next().ok_or(MALFORMED)?;
            if quote != '"' && quote != '\'' {
                return Err(MALFORMED);
            }
            self.at += 1;
            let end = self.rest().find(quote).ok_or(MALFORMED)?;
            let value = unescape(&self.rest()[..end])?;
            self.at += end + 1;
            node.attrs.push((key, value));
        }
    }
    /// The content of `node` up to its end tag.
    fn content(&mut self, node: &mut Node) -> Result<(), u32> {
        loop {
            if self.rest().is_empty() {
                return Err(MALFORMED);
            }
            if self.rest().starts_with("</") {
                self.at += 2;
                let name = self.name()?;
                self.skip_space();
                if name != node.name || !self.rest().starts_with('>') {
                    return Err(MALFORMED);
                }
                self.at += 1;
                return Ok(());
            }
            if self.rest().starts_with("<![CDATA[") {
                let end = self.rest().find("]]>").ok_or(MALFORMED)?;
                node.text.push_str(&self.rest()[9..end]);
                self.at += end + 3;
                continue;
            }
            if self.rest().starts_with("<!--") || self.rest().starts_with("<?") {
                self.skip_misc()?;
                continue;
            }
            if self.rest().starts_with('<') {
                let (mut child, empty) = self.start()?;
                if !empty {
                    self.content(&mut child)?;
                }
                node.children.push(child);
                continue;
            }
            let end = self.rest().find('<').ok_or(MALFORMED)?;
            node.text.push_str(&unescape(&self.rest()[..end])?);
            self.at += end;
        }
    }
}
fn unescape(s: &str) -> Result<String, u32> {
    let mut out = String::new();
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        let end = rest[i..].find(';').ok_or(MALFORMED)?;
        let entity = &rest[i + 1..i + end];
        let c = match entity {
            "amp" => '&',
            "lt" => '<',
            "gt" => '>',
            "quot" => '"',
            "apos" => '\'',
            e if e.starts_with("#x") => {
                char::from_u32(u32::from_str_radix(&e[2..], 16).map_err(|_| MALFORMED)?)
                    .ok_or(MALFORMED)?
            }
            e if e.starts_with('#') => {
                char::from_u32(e[1..].parse().map_err(|_| MALFORMED)?).ok_or(MALFORMED)?
            }
            _ => return Err(MALFORMED),
        };
        out.push(c);
        rest = &rest[i + end + 1..];
    }
    out.push_str(rest);
    Ok(out)
}
/// The XML's root element, checked for the task namespace before the rest
/// is read (Windows answers SCHED_E_NAMESPACE for `<Task>` alone).
pub fn parse_tree(text: &str) -> Result<Node, u32> {
    let mut r = Reader {
        s: text.trim_start_matches('\u{feff}'),
        at: 0,
    };
    r.skip_misc()?;
    let (mut root, empty) = r.start()?;
    if root.name != "Task" || root.attr("xmlns") != Some(NAMESPACE) {
        return Err(hr::SCHED_E_NAMESPACE);
    }
    if !empty {
        r.content(&mut root)?;
    }
    r.skip_misc()?;
    if !r.rest().is_empty() {
        return Err(MALFORMED);
    }
    Ok(root)
}
fn text(n: &Node) -> Option<String> {
    Some(String::from(n.text.trim()))
}
fn boolean(n: &Node) -> Result<bool, u32> {
    match n.text.trim() {
        "true" | "1" => Ok(true),
        "false" | "0" => Ok(false),
        _ => Err(hr::SCHED_E_INVALIDVALUE),
    }
}
fn number<T: core::str::FromStr>(n: &Node) -> Result<T, u32> {
    n.text.trim().parse().map_err(|_| hr::SCHED_E_INVALIDVALUE)
}
fn named(n: &Node, names: &[&str]) -> Result<usize, u32> {
    names
        .iter()
        .position(|x| *x == n.text.trim())
        .ok_or(hr::SCHED_E_INVALIDVALUE)
}
fn bits(n: &Node, names: &[&str]) -> Result<u32, u32> {
    let mut v = 0;
    for c in &n.children {
        v |= 1
            << names
                .iter()
                .position(|x| *x == c.name)
                .ok_or(hr::SCHED_E_UNEXPECTEDNODE)?;
    }
    Ok(v)
}
const UNEXPECTED: u32 = hr::SCHED_E_UNEXPECTEDNODE;
fn read_trigger(n: &Node) -> Result<Trigger, u32> {
    let kind = match n.name.as_str() {
        "TimeTrigger" => trigger::TIME,
        "CalendarTrigger" => {
            let by = n
                .children
                .iter()
                .find(|c| c.name.starts_with("ScheduleBy"))
                .ok_or(hr::SCHED_E_MISSINGNODE)?;
            match by.name.as_str() {
                "ScheduleByDay" => trigger::DAILY,
                "ScheduleByWeek" => trigger::WEEKLY,
                "ScheduleByMonth" => trigger::MONTHLY,
                "ScheduleByMonthDayOfWeek" => trigger::MONTHLYDOW,
                _ => return Err(UNEXPECTED),
            }
        }
        "IdleTrigger" => trigger::IDLE,
        "RegistrationTrigger" => trigger::REGISTRATION,
        "BootTrigger" => trigger::BOOT,
        "LogonTrigger" => trigger::LOGON,
        "SessionStateChangeTrigger" => trigger::SESSION_STATE_CHANGE,
        "EventTrigger" => trigger::EVENT,
        _ => return Err(UNEXPECTED),
    };
    let mut t = Trigger::new(kind);
    t.id = n.attr("id").map(String::from);
    for c in &n.children {
        match c.name.as_str() {
            "Repetition" => {
                for r in &c.children {
                    match r.name.as_str() {
                        "Interval" => t.repetition.interval = text(r),
                        "Duration" => t.repetition.duration = text(r),
                        "StopAtDurationEnd" => t.repetition.stop_at_end = boolean(r)?,
                        _ => return Err(UNEXPECTED),
                    }
                }
            }
            "StartBoundary" => t.start = text(c),
            "EndBoundary" => t.end = text(c),
            "ExecutionTimeLimit" => t.limit = text(c),
            "Enabled" => t.enabled = boolean(c)?,
            "RandomDelay" => t.random_delay = text(c),
            "Delay" => t.delay = text(c),
            "UserId" => t.user_id = text(c),
            "StateChange" => {
                t.state_change = STATE_CHANGES
                    .iter()
                    .find(|(_, s)| *s == c.text.trim())
                    .map(|(v, _)| *v)
                    .ok_or(hr::SCHED_E_INVALIDVALUE)?
            }
            "Subscription" => t.subscription = Some(c.text.clone()),
            "ValueQueries" => {
                for v in &c.children {
                    if v.name != "Value" {
                        return Err(UNEXPECTED);
                    }
                    t.value_queries
                        .push((String::from(v.attr("name").unwrap_or("")), v.text.clone()));
                }
            }
            "ScheduleByDay" => {
                for s in &c.children {
                    match s.name.as_str() {
                        "DaysInterval" => t.days_interval = number(s)?,
                        _ => return Err(UNEXPECTED),
                    }
                }
            }
            "ScheduleByWeek" => {
                for s in &c.children {
                    match s.name.as_str() {
                        "DaysOfWeek" => t.days_of_week = bits(s, &DAYS)? as i16,
                        "WeeksInterval" => t.weeks_interval = number(s)?,
                        _ => return Err(UNEXPECTED),
                    }
                }
            }
            "ScheduleByMonth" => {
                for s in &c.children {
                    match s.name.as_str() {
                        "DaysOfMonth" => {
                            for d in &s.children {
                                match d.text.trim() {
                                    "Last" => t.last_day = true,
                                    _ => {
                                        let day: u32 = number(d)?;
                                        if !(1..=31).contains(&day) {
                                            return Err(hr::SCHED_E_INVALIDVALUE);
                                        }
                                        t.days_of_month |= 1 << (day - 1);
                                    }
                                }
                            }
                        }
                        "Months" => t.months = bits(s, &MONTHS)? as i16,
                        _ => return Err(UNEXPECTED),
                    }
                }
            }
            "ScheduleByMonthDayOfWeek" => {
                for s in &c.children {
                    match s.name.as_str() {
                        "Weeks" => {
                            for wk in &s.children {
                                match wk.text.trim() {
                                    "Last" => t.last_week = true,
                                    _ => {
                                        let week: u32 = number(wk)?;
                                        if !(1..=4).contains(&week) {
                                            return Err(hr::SCHED_E_INVALIDVALUE);
                                        }
                                        t.weeks_of_month |= 1 << (week - 1);
                                    }
                                }
                            }
                        }
                        "DaysOfWeek" => t.days_of_week = bits(s, &DAYS)? as i16,
                        "Months" => t.months = bits(s, &MONTHS)? as i16,
                        _ => return Err(UNEXPECTED),
                    }
                }
            }
            _ => return Err(UNEXPECTED),
        }
    }
    Ok(t)
}
fn read_action(n: &Node) -> Result<Action, u32> {
    let kind = match n.name.as_str() {
        "Exec" => action::EXEC,
        "ComHandler" => action::COM_HANDLER,
        "SendEmail" => action::SEND_EMAIL,
        "ShowMessage" => action::SHOW_MESSAGE,
        _ => return Err(UNEXPECTED),
    };
    let mut a = Action::new(kind);
    a.id = n.attr("id").map(String::from);
    for c in &n.children {
        let v = || Some(c.text.clone());
        match (kind, c.name.as_str()) {
            (action::EXEC, "Command") => a.path = text(c),
            (action::EXEC, "Arguments") => a.arguments = v(),
            (action::EXEC, "WorkingDirectory") => a.directory = text(c),
            (action::COM_HANDLER, "ClassId") => a.class_id = text(c),
            (action::COM_HANDLER, "Data") => a.data = v(),
            (action::SEND_EMAIL, "Server") => a.server = v(),
            (action::SEND_EMAIL, "Subject") => a.subject = v(),
            (action::SEND_EMAIL, "To") => a.to = v(),
            (action::SEND_EMAIL, "Cc") => a.cc = v(),
            (action::SEND_EMAIL, "Bcc") => a.bcc = v(),
            (action::SEND_EMAIL, "ReplyTo") => a.reply_to = v(),
            (action::SEND_EMAIL, "From") => a.from = v(),
            (action::SEND_EMAIL | action::SHOW_MESSAGE, "Body") => a.body = v(),
            (action::SEND_EMAIL, "HeaderFields") => {
                for f in &c.children {
                    let field = |n: &str| {
                        f.children
                            .iter()
                            .find(|x| x.name == n)
                            .map(|x| x.text.clone())
                            .unwrap_or_default()
                    };
                    a.header_fields.push((field("Name"), field("Value")));
                }
            }
            (action::SEND_EMAIL, "Attachments") => {}
            (action::SHOW_MESSAGE, "Title") => a.title = v(),
            _ => return Err(UNEXPECTED),
        }
    }
    Ok(a)
}
/// A definition from task XML (ITaskDefinition::put_XmlText, RegisterTask).
pub fn read(xml: &str) -> Result<Definition, u32> {
    let root = parse_tree(xml)?;
    let mut d = Definition::from_defaults();
    if let Some(v) = root.attr("version") {
        let minor = v
            .strip_prefix("1.")
            .and_then(|m| m.parse::<u32>().ok())
            .ok_or(hr::SCHED_E_INVALIDVALUE)?;
        d.settings.compatibility = minor;
    }
    for part in &root.children {
        match part.name.as_str() {
            "RegistrationInfo" => {
                for c in &part.children {
                    let slot = match c.name.as_str() {
                        "Source" => &mut d.info.source,
                        "Date" => &mut d.info.date,
                        "Author" => &mut d.info.author,
                        "Version" => &mut d.info.version,
                        "Description" => &mut d.info.description,
                        "Documentation" => &mut d.info.documentation,
                        "URI" => &mut d.info.uri,
                        "SecurityDescriptor" => &mut d.info.security_descriptor,
                        _ => return Err(UNEXPECTED),
                    };
                    *slot = Some(c.text.clone());
                }
            }
            "Triggers" => {
                for c in &part.children {
                    d.triggers.push(read_trigger(c)?);
                }
            }
            "Principals" => {
                for p in &part.children {
                    if p.name != "Principal" {
                        return Err(UNEXPECTED);
                    }
                    d.principal.id = p.attr("id").map(String::from);
                    for c in &p.children {
                        match c.name.as_str() {
                            "UserId" => d.principal.user_id = text(c),
                            "GroupId" => d.principal.group_id = text(c),
                            "DisplayName" => d.principal.display_name = Some(c.text.clone()),
                            "LogonType" => {
                                d.principal.logon = named(c, &LOGON_TYPES)? as u32;
                                d.principal.logon_set = true;
                            }
                            "RunLevel" => {
                                d.principal.run_level =
                                    named(c, &["LeastPrivilege", "HighestAvailable"])? as u32
                            }
                            "ProcessTokenSidType" | "RequiredPrivileges" => {}
                            _ => return Err(UNEXPECTED),
                        }
                    }
                }
            }
            "Settings" => {
                let s = &mut d.settings;
                for c in &part.children {
                    match c.name.as_str() {
                        "MultipleInstancesPolicy" => s.instances = named(c, &INSTANCES)? as u32,
                        "DisallowStartIfOnBatteries" => s.disallow_on_batteries = boolean(c)?,
                        "StopIfGoingOnBatteries" => s.stop_on_batteries = boolean(c)?,
                        "AllowHardTerminate" => s.hard_terminate = boolean(c)?,
                        "StartWhenAvailable" => s.when_available = boolean(c)?,
                        "RunOnlyIfNetworkAvailable" => s.network_only = boolean(c)?,
                        "NetworkSettings" => {
                            for n in &c.children {
                                match n.name.as_str() {
                                    "Name" => s.network_name = Some(n.text.clone()),
                                    "Id" => s.network_id = text(n),
                                    _ => return Err(UNEXPECTED),
                                }
                            }
                        }
                        "IdleSettings" => {
                            for n in &c.children {
                                match n.name.as_str() {
                                    "Duration" => s.idle.duration = text(n),
                                    "WaitTimeout" => s.idle.wait = text(n),
                                    "StopOnIdleEnd" => s.idle.stop_on_end = boolean(n)?,
                                    "RestartOnIdle" => s.idle.restart = boolean(n)?,
                                    _ => return Err(UNEXPECTED),
                                }
                            }
                        }
                        "AllowStartOnDemand" => s.demand_start = boolean(c)?,
                        "Enabled" => s.enabled = boolean(c)?,
                        "Hidden" => s.hidden = boolean(c)?,
                        "RunOnlyIfIdle" => s.idle_only = boolean(c)?,
                        "DisallowStartOnRemoteAppSession" => s.remote_app = boolean(c)?,
                        "UseUnifiedSchedulingEngine" => s.unified = boolean(c)?,
                        "WakeToRun" => s.wake = boolean(c)?,
                        "ExecutionTimeLimit" => s.limit = text(c),
                        "DeleteExpiredTaskAfter" => s.delete_after = text(c),
                        "Priority" => {
                            s.priority = number(c)?;
                            if !(0..=10).contains(&s.priority) {
                                return Err(hr::SCHED_E_INVALIDVALUE);
                            }
                        }
                        "RestartOnFailure" => {
                            for n in &c.children {
                                match n.name.as_str() {
                                    "Interval" => s.restart_interval = text(n),
                                    "Count" => s.restart_count = number(n)?,
                                    _ => return Err(UNEXPECTED),
                                }
                            }
                        }
                        "Volatile" => s.volatile = boolean(c)?,
                        "MaintenanceSettings" => {}
                        _ => return Err(UNEXPECTED),
                    }
                }
            }
            "Data" => d.data = Some(part.text.clone()),
            "Actions" => {
                d.context = part.attr("Context").map(String::from);
                for c in &part.children {
                    d.actions.push(read_action(c)?);
                }
            }
            _ => return Err(UNEXPECTED),
        }
    }
    Ok(d)
}
