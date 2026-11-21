//! The console tools Windows ships in System32 that batch files use
//! (find, findstr, sort, more, fc, comp, tree, attrib, xcopy, robocopy,
//! where, whoami, hostname, ipconfig, ping, tasklist, taskkill, timeout,
//! choice, clip, systeminfo, reg, setx, certutil, chcp, mode, net, sc,
//! nslookup, netstat, shutdown and the administrator tools), built into the
//! engine with Windows' output formats. `where find` reports them in
//! C:\Windows\System32, where Windows has them.
use super::Cmd;
use super::builtin::{Outcome, Pending as BuiltinPending, decode_text, tokens, unq};
use crate::format::{date, date_time_full, grouped, time_seconds};
use crate::host::{Echo, FsError, Host, Meta, RegValue, attr};
use crate::{InputKind, Resume, Step, wild, wpath};
use alloc::{format, string::String, vec, vec::Vec};

pub(crate) const NAMES: &[&str] = &[
    "attrib",
    "bcdedit",
    "cacls",
    "certutil",
    "chcp",
    "chkdsk",
    "chkntfs",
    "choice",
    "cipher",
    "clip",
    "cmd",
    "comp",
    "compact",
    "convert",
    "diskpart",
    "dism",
    "doskey",
    "driverquery",
    "fc",
    "find",
    "findstr",
    "forfiles",
    "format",
    "fsutil",
    "eventcreate",
    "getmac",
    "gpresult",
    "help",
    "hostname",
    "icacls",
    "ipconfig",
    "label",
    "mode",
    "more",
    "msg",
    "net",
    "netsh",
    "netstat",
    "nslookup",
    "openfiles",
    "ping",
    "print",
    "recover",
    "reg",
    "replace",
    "robocopy",
    "route",
    "sc",
    "schtasks",
    "setx",
    "sfc",
    "shutdown",
    "sort",
    "subst",
    "systeminfo",
    "takeown",
    "taskkill",
    "tasklist",
    "timeout",
    "tracert",
    "tree",
    "wevtutil",
    "waitfor",
    "where",
    "whoami",
    "wmic",
    "xcopy",
    "arp",
    "logoff",
    "rasdial",
    "wireguard",
];
pub(crate) enum Pending {
    Choice {
        keys: Vec<char>,
        sensitive: bool,
        default: Option<char>,
        remaining: u64,
    },
    Timeout {
        remaining: u64,
        nobreak: bool,
    },
    More {
        lines: Vec<String>,
        at: usize,
    },
    Ping(PingState),
    Trace(TraceState),
    Comp,
    Dial(super::vpn::Dial),
}
pub(crate) struct PingState {
    name: String,
    address: [u8; 4],
    bytes: usize,
    count: Option<u32>,
    sent: u32,
    received: u32,
    times: Vec<u64>,
    timeout: u64,
    ttl: u8,
}

fn input_text(cmd: &Cmd) -> Option<String> {
    cmd.input().map(|i| decode_text(&i))
}
fn done(ok: bool) -> Outcome {
    Outcome::Done(ok)
}
fn err(cmd: &mut Cmd, host: &mut dyn Host, text: &str, code: i32) -> Outcome {
    cmd.err_line(host, text);
    cmd.errorlevel = code;
    Outcome::Done(false)
}
/// Administrator tools as a standard user's console sees them.
fn denied_elevation(cmd: &mut Cmd, host: &mut dyn Host) -> Outcome {
    err(
        cmd,
        host,
        "Access Denied as you do not have sufficient privileges or\nthe disk may be locked by another process.\nYou have to invoke this utility running in elevated mode\nand make sure the disk is unlocked.",
        1,
    )
}

pub(crate) fn run(cmd: &mut Cmd, host: &mut dyn Host, tool: &str, tail: &str) -> Outcome {
    let args = tokens(tail);
    cmd.errorlevel = 0;
    match tool {
        "find" => find(cmd, host, tail),
        "findstr" => findstr(cmd, host, &args),
        "sort" => sort(cmd, host, &args),
        "more" => more(cmd, host, &args),
        "fc" => fc(cmd, host, &args),
        "comp" => comp(cmd, host, &args),
        "tree" => tree(cmd, host, &args),
        "attrib" => attrib(cmd, host, &args),
        "xcopy" => xcopy(cmd, host, &args),
        "robocopy" => robocopy(cmd, host, &args),
        "replace" => xcopy(cmd, host, &args),
        "where" => where_(cmd, host, &args),
        "whoami" => whoami(cmd, host, &args),
        "hostname" => {
            let n = host.computer_name();
            cmd.out_line(host, &n);
            done(true)
        }
        "ipconfig" => ipconfig(cmd, host, &args),
        "rasdial" => super::vpn::rasdial(cmd, host, &args),
        "wireguard" => super::vpn::wireguard(cmd, host, &args),
        "getmac" => getmac(cmd, host),
        "ping" => ping(cmd, host, &args),
        "tracert" => tracert(cmd, host, &args),
        "nslookup" => nslookup(cmd, host, &args),
        "netstat" => netstat(cmd, host, &args),
        "arp" => arp(cmd, host, &args),
        "route" => {
            cmd.out_line(host, "===========================================================================\nIPv4 Route Table\n===========================================================================\nActive Routes:\nNetwork Destination        Netmask          Gateway       Interface  Metric");
            for a in host.adapters().into_iter().filter(|a| a.ipv4.is_some()) {
                if let Some(g) = a.gateway {
                    cmd.out_line(
                        host,
                        &format!(
                            "          0.0.0.0          0.0.0.0{:>17}{:>16}     25",
                            ip4(g),
                            ip4(a.ipv4.unwrap())
                        ),
                    );
                }
            }
            cmd.out_line(
                host,
                "===========================================================================",
            );
            done(true)
        }
        "tasklist" => tasklist(cmd, host, &args),
        "taskkill" => taskkill(cmd, host, &args),
        "timeout" => timeout(cmd, host, &args),
        "waitfor" => err(cmd, host, "ERROR: Timed out waiting for the signal.", 1),
        "choice" => choice(cmd, host, &args),
        "clip" => {
            let text = input_text(cmd).unwrap_or_default();
            match host.set_clipboard(&text) {
                Ok(()) => done(true),
                Err(_) => err(cmd, host, "ERROR: The clipboard isn't available.", 1),
            }
        }
        "systeminfo" => systeminfo(cmd, host),
        "reg" => reg(cmd, host, &args),
        "setx" => setx(cmd, host, &args),
        "certutil" => certutil(cmd, host, &args),
        "chcp" => {
            if let Some(cp) = args.first().and_then(|a| a.parse::<u32>().ok()) {
                if ![437, 850, 852, 866, 936, 1252, 65001].contains(&cp) {
                    return err(cmd, host, "Invalid code page", 1);
                }
                cmd.codepage = cp;
            }
            let cp = cmd.codepage;
            cmd.out_line(host, &format!("Active code page: {cp}"));
            done(true)
        }
        "mode" => {
            let mut cols = None;
            let mut lines = None;
            for a in &args {
                let l = a.to_ascii_lowercase();
                if let Some(v) = l.strip_prefix("cols") {
                    cols = v.trim_start_matches('=').parse::<usize>().ok();
                }
                if let Some(v) = l.strip_prefix("lines") {
                    lines = v.trim_start_matches('=').parse::<usize>().ok();
                }
            }
            if cols.is_some() || lines.is_some() {
                let (c, r) = (cols.unwrap_or(cmd.columns), lines.unwrap_or(cmd.rows));
                cmd.set_size(c, r);
                return done(true);
            }
            let (rows, columns, cp) = (cmd.rows, cmd.columns, cmd.codepage);
            cmd.out_line(host, &format!("\nStatus for device CON:\n----------------------\n    Lines:          {rows}\n    Columns:        {columns}\n    Keyboard rate:  31\n    Keyboard delay: 1\n    Code page:      {cp}\n"));
            done(true)
        }
        "help" => {
            let text = match args.first() {
                Some(c) => super::help::text(&unq(c)),
                None => super::help::list(),
            };
            cmd.out(host, &text);
            done(true)
        }
        "cmd" => nested(cmd, host, tail),
        "doskey" => done(true),
        "sfc" | "dism" | "fsutil" | "chkdsk" if host.elevated() => {
            elevated_tool(cmd, host, tool, &args)
        }
        "label" | "format" | "chkdsk" | "chkntfs" | "convert" | "recover" | "diskpart" | "sfc"
        | "dism" | "bcdedit" | "fsutil" => {
            if tool == "bcdedit" {
                return err(
                    cmd,
                    host,
                    "The boot configuration data store could not be opened.\nAccess is denied.",
                    1,
                );
            }
            if tool == "sfc" {
                return err(
                    cmd,
                    host,
                    "You must be an administrator running a console session in order to\nuse the sfc utility.",
                    1,
                );
            }
            if tool == "dism" {
                return err(
                    cmd,
                    host,
                    "\nError: 740\n\nElevated permissions are required to run DISM.\nUse an elevated command prompt to complete these tasks.",
                    740,
                );
            }
            if tool == "fsutil" {
                return err(
                    cmd,
                    host,
                    "The FSUTIL utility requires that you have administrative privileges.",
                    1,
                );
            }
            denied_elevation(cmd, host)
        }
        "compact" | "cipher" => {
            let cwd = cmd.cwd.clone();
            cmd.out_line(host, &format!("\n Listing {cwd}\\\n New files added to this directory will not be compressed.\n"));
            done(true)
        }
        "icacls" | "cacls" => {
            let target = args
                .iter()
                .find(|a| !a.starts_with('/'))
                .map(|a| unq(a))
                .unwrap_or_default();
            let Ok(full) = cmd.full(&target) else {
                return err(cmd, host, "The system cannot find the file specified.", 2);
            };
            if host.native(&full).is_none_or(|n| host.stat(&n).is_err()) {
                return err(
                    cmd,
                    host,
                    &format!(
                        "{target}: The system cannot find the file specified.\nSuccessfully processed 0 files; Failed processing 1 files"
                    ),
                    2,
                );
            }
            let user = format!("{}\\{}", host.domain(), host.user_name());
            cmd.out_line(host, &format!("{full} NT AUTHORITY\\SYSTEM:(I)(F)\n{:width$}BUILTIN\\Administrators:(I)(F)\n{:width$}{user}:(I)(F)\n", "", "", width = full.len() + 1));
            cmd.out_line(
                host,
                "Successfully processed 1 files; Failed processing 0 files",
            );
            done(true)
        }
        "takeown" => err(cmd, host, "ERROR: Access is denied.", 1),
        "driverquery" => {
            cmd.out_line(host, "\nModule Name  Display Name           Driver Type   Link Date\n============ ====================== ============= ======================");
            for (m, d) in [
                ("door_blk", "Doors VirtIO Block"),
                ("door_net", "Doors VirtIO Network"),
                ("door_snd", "Doors HD Audio"),
                ("door_gpu", "Doors Display"),
            ] {
                cmd.out_line(host, &format!("{m:<13}{d:<23}Kernel        "));
            }
            done(true)
        }
        "gpresult" => err(cmd, host, "\nINFO: The user does not have RSoP data.", 1),
        "openfiles" => err(
            cmd,
            host,
            "ERROR: Logged-on user does not have administrative privilege.",
            1,
        ),
        "print" => err(cmd, host, "Unable to initialize device PRN", 1),
        "subst" => {
            if args.is_empty() {
                return done(true);
            }
            err(
                cmd,
                host,
                "Invalid parameter - the drive letter can't be assigned.",
                1,
            )
        }
        "forfiles" => forfiles(cmd, host, &args),
        "net" => net(cmd, host, &args),
        "netsh" => super::netsh::netsh(cmd, host, tail),
        "sc" => super::sc::sc(cmd, host, &args),
        "schtasks" => super::schtasks::schtasks(cmd, host, &args),
        "eventcreate" => super::events::eventcreate(cmd, host, &args),
        "wevtutil" => super::events::wevtutil(cmd, host, &args),
        "shutdown" => {
            let lower: Vec<String> = args.iter().map(|a| a.to_ascii_lowercase()).collect();
            let action = if lower.iter().any(|a| a == "/r" || a == "-r") {
                Some(crate::Launch::Power(crate::Power::Restart))
            } else if lower.iter().any(|a| a == "/s" || a == "-s" || a == "/p") {
                Some(crate::Launch::Power(crate::Power::ShutDown))
            } else if lower.iter().any(|a| a == "/l" || a == "-l") {
                Some(crate::Launch::Power(crate::Power::SignOut))
            } else if lower.iter().any(|a| a == "/a" || a == "-a") {
                return err(
                    cmd,
                    host,
                    "Unable to abort the system shutdown because no shutdown was in progress.(1116)",
                    1116,
                );
            } else {
                None
            };
            match action {
                Some(a) => Outcome::Step(Step::Launch(a)),
                None => {
                    cmd.out(host, &super::help::text("shutdown"));
                    done(true)
                }
            }
        }
        "logoff" => Outcome::Step(Step::Launch(crate::Launch::Power(crate::Power::SignOut))),
        "msg" => err(cmd, host, "Error 5 getting session names", 1),
        "wmic" => wmic(cmd, host, &args),
        _ => err(
            cmd,
            host,
            &format!(
                "'{tool}' is not recognized as an internal or external command,\noperable program or batch file."
            ),
            9009,
        ),
    }
}

pub(crate) fn resume(cmd: &mut Cmd, host: &mut dyn Host, p: Pending, r: Resume) -> Outcome {
    match p {
        Pending::Choice {
            keys,
            sensitive,
            default,
            remaining,
        } => {
            let pick = match r {
                Resume::Key(c) => Some(c),
                Resume::Line(l) => l.chars().next(),
                Resume::Tick => {
                    if remaining <= 1 {
                        default
                    } else {
                        return Outcome::Wait(
                            BuiltinPending::Tool(Pending::Choice {
                                keys,
                                sensitive,
                                default,
                                remaining: remaining - 1,
                            }),
                            Step::Sleep {
                                ms: 1000,
                                keys: true,
                            },
                        );
                    }
                }
                _ => None,
            };
            let Some(c) = pick else {
                return Outcome::Wait(
                    BuiltinPending::Tool(Pending::Choice {
                        keys,
                        sensitive,
                        default,
                        remaining,
                    }),
                    Step::Input(InputKind::Key),
                );
            };
            let index = keys.iter().position(|k| {
                if sensitive {
                    *k == c
                } else {
                    k.eq_ignore_ascii_case(&c)
                }
            });
            match index {
                Some(i) => {
                    cmd.out_line(host, &String::from(keys[i]));
                    cmd.errorlevel = i as i32 + 1;
                    done(true)
                }
                None => {
                    // Windows beeps and keeps waiting.
                    let step = if default.is_some() {
                        Step::Sleep {
                            ms: 1000,
                            keys: true,
                        }
                    } else {
                        Step::Input(InputKind::Key)
                    };
                    Outcome::Wait(
                        BuiltinPending::Tool(Pending::Choice {
                            keys,
                            sensitive,
                            default,
                            remaining,
                        }),
                        step,
                    )
                }
            }
        }
        Pending::Timeout { remaining, nobreak } => match r {
            Resume::Key(_) | Resume::Line(_) if !nobreak => {
                cmd.out(host, "\n");
                done(true)
            }
            _ => {
                if remaining <= 1 {
                    cmd.out(host, "\n");
                    return done(true);
                }
                Outcome::Wait(
                    BuiltinPending::Tool(Pending::Timeout {
                        remaining: remaining - 1,
                        nobreak,
                    }),
                    Step::Sleep {
                        ms: 1000,
                        keys: !nobreak,
                    },
                )
            }
        },
        Pending::More { lines, at } => {
            let page = cmd.rows.saturating_sub(1).max(1);
            let step = match r {
                Resume::Key('q' | 'Q') => return done(true),
                Resume::Key('\r' | '\n') | Resume::Line(_) => 1,
                _ => page,
            };
            cmd.out(host, "\r           \r");
            more_page(cmd, host, lines, at, step)
        }
        Pending::Ping(s) => ping_next(cmd, host, s),
        Pending::Trace(s) => trace_next(cmd, host, s),
        Pending::Dial(d) => super::vpn::dial_tick(cmd, host, d),
        Pending::Comp => {
            let again = matches!(r, Resume::Key('y' | 'Y'));
            cmd.out_line(host, if again { "Y" } else { "N" });
            done(true)
        }
    }
}

// ----- Text tools -----
fn file_list(
    cmd: &mut Cmd,
    host: &mut dyn Host,
    names: &[String],
) -> Vec<(String, Option<String>)> {
    // (display name, native path or None when missing)
    let mut out = Vec::new();
    for n in names {
        let n = unq(n);
        if wpath::wild(&n) {
            for (p, _) in cmd.glob(host, &n, false) {
                let native = host.native(&p);
                out.push((String::from(wpath::name(&p)), native));
            }
        } else {
            let native = cmd
                .native(host, &n)
                .ok()
                .filter(|p| host.stat(p).is_ok_and(|m| !m.dir));
            out.push((n, native));
        }
    }
    out
}
fn find(cmd: &mut Cmd, host: &mut dyn Host, tail: &str) -> Outcome {
    // FIND's string is the first quoted argument.
    let mut opts = (false, false, false, false); // v, c, n, i
    let Some(q1) = tail.find('"') else {
        return err(cmd, host, "FIND: Parameter format not correct", 2);
    };
    let Some(q2) = tail[q1 + 1..].find('"').map(|p| q1 + 1 + p) else {
        return err(cmd, host, "FIND: Parameter format not correct", 2);
    };
    let needle = &tail[q1 + 1..q2];
    for s in tokens(&tail[..q1]).into_iter().chain(
        tokens(&tail[q2 + 1..])
            .into_iter()
            .filter(|t| t.starts_with('/')),
    ) {
        match s.to_ascii_lowercase().as_str() {
            "/v" => opts.0 = true,
            "/c" => opts.1 = true,
            "/n" => opts.2 = true,
            "/i" => opts.3 = true,
            "/off" | "/offline" => (),
            other => {
                return err(
                    cmd,
                    host,
                    &format!("FIND: Invalid switch {}", other.to_ascii_uppercase()),
                    2,
                );
            }
        }
    }
    let names: Vec<String> = tokens(&tail[q2 + 1..])
        .into_iter()
        .filter(|t| !t.starts_with('/'))
        .collect();
    let matches = |line: &str| {
        let hit = if opts.3 {
            line.to_lowercase().contains(&needle.to_lowercase())
        } else {
            line.contains(needle)
        };
        hit != opts.0
    };
    let mut found = false;
    let mut report = |cmd: &mut Cmd, host: &mut dyn Host, header: Option<&str>, text: &str| {
        let hits: Vec<(usize, &str)> = text
            .lines()
            .enumerate()
            .filter(|(_, l)| matches(l))
            .collect();
        found |= !hits.is_empty();
        if opts.1 {
            match header {
                Some(h) => cmd.out_line(
                    host,
                    &format!("\n---------- {}: {}", h.to_uppercase(), hits.len()),
                ),
                None => cmd.out_line(host, &format!("{}", hits.len())),
            }
            return;
        }
        if let Some(h) = header {
            cmd.out_line(host, &format!("\n---------- {}", h.to_uppercase()));
        }
        for (i, l) in hits {
            if opts.2 {
                cmd.out_line(host, &format!("[{}]{l}", i + 1));
            } else {
                cmd.out_line(host, l);
            }
        }
    };
    if names.is_empty() {
        let text = input_text(cmd).unwrap_or_default();
        report(cmd, host, None, &text);
    } else {
        for (display, native) in file_list(cmd, host, &names) {
            match native.and_then(|n| host.read(&n, 16 << 20).ok()) {
                Some(b) => report(cmd, host, Some(&display), &decode_text(&b)),
                None => cmd.err_line(
                    host,
                    &format!("File not found - {}", display.to_uppercase()),
                ),
            }
        }
    }
    cmd.errorlevel = if found { 0 } else { 1 };
    done(found)
}
/// findstr's regular expressions: `.` `*` `^` `$` `[class]` `[^class]`
/// `[x-y]` `\<` `\>` and `\x` escapes.
#[derive(Clone, Debug)]
enum Atom {
    Char(char),
    Any,
    Class(Vec<(char, char)>, bool),
    Start,
    End,
    WordStart,
    WordEnd,
}
fn compile(pattern: &str) -> Vec<(Atom, bool)> {
    let c: Vec<char> = pattern.chars().collect();
    let mut out: Vec<(Atom, bool)> = Vec::new();
    let mut i = 0;
    while i < c.len() {
        let atom = match c[i] {
            '.' => Atom::Any,
            '^' if i == 0 => Atom::Start,
            '$' if i + 1 == c.len() => Atom::End,
            '\\' if i + 1 < c.len() => {
                i += 1;
                match c[i] {
                    '<' => Atom::WordStart,
                    '>' => Atom::WordEnd,
                    x => Atom::Char(x),
                }
            }
            '[' => {
                let mut j = i + 1;
                let neg = c.get(j) == Some(&'^');
                if neg {
                    j += 1;
                }
                let mut ranges = Vec::new();
                while j < c.len() && c[j] != ']' {
                    if j + 2 < c.len() && c[j + 1] == '-' && c[j + 2] != ']' {
                        ranges.push((c[j], c[j + 2]));
                        j += 3;
                    } else {
                        ranges.push((c[j], c[j]));
                        j += 1;
                    }
                }
                i = j;
                Atom::Class(ranges, neg)
            }
            x => Atom::Char(x),
        };
        i += 1;
        let star = c.get(i) == Some(&'*');
        if star {
            i += 1;
        }
        out.push((atom, star));
    }
    out
}
fn atom_matches(a: &Atom, ch: char, insensitive: bool) -> bool {
    let eq = |x: char, y: char| {
        if insensitive {
            x.to_lowercase().eq(y.to_lowercase())
        } else {
            x == y
        }
    };
    match a {
        Atom::Char(x) => eq(*x, ch),
        Atom::Any => true,
        Atom::Class(ranges, neg) => {
            let lc = if insensitive {
                ch.to_ascii_lowercase()
            } else {
                ch
            };
            let hit = ranges.iter().any(|(a, b)| {
                (*a..=*b).contains(&ch)
                    || (insensitive
                        && ((a.to_ascii_lowercase()..=b.to_ascii_lowercase()).contains(&lc)
                            || (a.to_ascii_uppercase()..=b.to_ascii_uppercase())
                                .contains(&ch.to_ascii_uppercase())))
            });
            hit != *neg
        }
        _ => false,
    }
}
fn match_here(p: &[(Atom, bool)], s: &[char], at: usize, insensitive: bool) -> Option<usize> {
    let Some((atom, star)) = p.first() else {
        return Some(at);
    };
    let word = |c: char| c.is_alphanumeric() || c == '_';
    match atom {
        Atom::Start => {
            if at == 0 {
                match_here(&p[1..], s, at, insensitive)
            } else {
                None
            }
        }
        Atom::End => {
            if at == s.len() {
                Some(at)
            } else {
                None
            }
        }
        Atom::WordStart => {
            let ok = at < s.len() && word(s[at]) && (at == 0 || !word(s[at - 1]));
            if ok {
                match_here(&p[1..], s, at, insensitive)
            } else {
                None
            }
        }
        Atom::WordEnd => {
            let ok = at > 0 && word(s[at - 1]) && (at == s.len() || !word(s[at]));
            if ok {
                match_here(&p[1..], s, at, insensitive)
            } else {
                None
            }
        }
        _ if *star => {
            let mut n = at;
            while n < s.len() && atom_matches(atom, s[n], insensitive) {
                n += 1;
            }
            loop {
                if let Some(e) = match_here(&p[1..], s, n, insensitive) {
                    return Some(e);
                }
                if n == at {
                    return None;
                }
                n -= 1;
            }
        }
        _ => {
            if at < s.len() && atom_matches(atom, s[at], insensitive) {
                match_here(&p[1..], s, at + 1, insensitive)
            } else {
                None
            }
        }
    }
}
/// Whether `line` matches (and the first match's start).
fn regex_find(p: &[(Atom, bool)], line: &str, insensitive: bool) -> Option<usize> {
    let s: Vec<char> = line.chars().collect();
    (0..=s.len()).find(|&start| match_here(p, &s, start, insensitive).is_some())
}
fn findstr(cmd: &mut Cmd, host: &mut dyn Host, args: &[String]) -> Outcome {
    let (
        mut begin,
        mut end,
        mut literal,
        mut subdirs,
        mut ins,
        mut whole,
        mut invert,
        mut numbers,
        mut names_only,
        mut offsets,
    ) = (
        false, false, false, false, false, false, false, false, false, false,
    );
    let mut strings: Vec<String> = Vec::new();
    let mut explicit = false;
    let mut rest: Vec<String> = Vec::new();
    for a in args {
        let lower = a.to_ascii_lowercase();
        if let Some(s) = lower.strip_prefix('/') {
            if let Some(v) = a.strip_prefix("/C:").or_else(|| a.strip_prefix("/c:")) {
                strings.push(unq(v));
                explicit = true;
                literal = true;
                continue;
            }
            if s.starts_with("g:")
                || s.starts_with("f:")
                || s.starts_with("d:")
                || s.starts_with("a:")
            {
                continue;
            }
            for f in s.chars() {
                match f {
                    'b' => begin = true,
                    'e' => end = true,
                    'l' => literal = true,
                    'r' => literal = false,
                    's' => subdirs = true,
                    'i' => ins = true,
                    'x' => whole = true,
                    'v' => invert = true,
                    'n' => numbers = true,
                    'm' => names_only = true,
                    'o' => offsets = true,
                    'p' | 'a' => (),
                    _ => return err(cmd, host, &format!("FINDSTR: /{f} ignored"), 2),
                }
            }
        } else {
            rest.push(a.clone());
        }
    }
    if !explicit {
        if rest.is_empty() {
            return err(cmd, host, "FINDSTR: Bad command line", 2);
        }
        // Space-separated words are separate search strings.
        strings = unq(&rest.remove(0))
            .split(' ')
            .filter(|s| !s.is_empty())
            .map(String::from)
            .collect();
    }
    let patterns: Vec<Vec<(Atom, bool)>> = strings
        .iter()
        .map(|s| {
            let mut text = if literal {
                s.chars()
                    .flat_map(|c| {
                        if "\\.*^$[".contains(c) {
                            vec!['\\', c]
                        } else {
                            vec![c]
                        }
                    })
                    .collect()
            } else {
                s.clone()
            };
            if (begin || whole) && !text.starts_with('^') {
                text.insert(0, '^');
            }
            if (end || whole) && !text.ends_with('$') {
                text.push('$');
            }
            compile(&text)
        })
        .collect();
    let hit = |line: &str| patterns.iter().any(|p| regex_find(p, line, ins).is_some()) != invert;
    let mut files: Vec<(String, Option<String>)> = Vec::new();
    for n in &rest {
        let n = unq(n);
        if subdirs {
            let (dir, pat) = match cmd.full(&n) {
                Ok(f) => {
                    let (d, p) = wpath::split(&f);
                    (String::from(d), String::from(p))
                }
                Err(_) => continue,
            };
            walk_files(host, &dir, &pat, &dir, &mut files);
        } else {
            files.extend(file_list(cmd, host, &[n]));
        }
    }
    let prefix = files.len() > 1 || subdirs || rest.iter().any(|r| wpath::wild(r));
    let mut found = false;
    if rest.is_empty() {
        let text = input_text(cmd).unwrap_or_default();
        for (i, line) in text.lines().enumerate() {
            if hit(line) {
                found = true;
                let n = if numbers {
                    format!("{}:", i + 1)
                } else {
                    String::new()
                };
                cmd.out_line(host, &format!("{n}{line}"));
            }
        }
    }
    for (display, native) in files {
        let Some(bytes) = native.and_then(|n| host.read(&n, 16 << 20).ok()) else {
            cmd.err_line(host, &format!("FINDSTR: Cannot open {display}"));
            continue;
        };
        let text = decode_text(&bytes);
        let mut offset = 0usize;
        for (i, line) in text.lines().enumerate() {
            if hit(line) {
                found = true;
                if names_only {
                    cmd.out_line(host, &display);
                    break;
                }
                let mut out = String::new();
                if prefix {
                    out.push_str(&format!("{display}:"));
                }
                if numbers {
                    out.push_str(&format!("{}:", i + 1));
                }
                if offsets {
                    out.push_str(&format!("{offset}:"));
                }
                out.push_str(line);
                cmd.out_line(host, &out);
            }
            offset += line.len() + 2;
        }
    }
    cmd.errorlevel = if found { 0 } else { 1 };
    done(found)
}
fn walk_files(
    host: &mut dyn Host,
    dir: &str,
    pattern: &str,
    base: &str,
    out: &mut Vec<(String, Option<String>)>,
) {
    let Some(native) = host.native(dir) else {
        return;
    };
    let Ok(mut entries) = host.list(&native) else {
        return;
    };
    entries.sort_by_key(|e| e.name.to_lowercase());
    for e in &entries {
        let full = wpath::join(dir, &e.name);
        if !e.meta.dir && wild::matches(pattern, &e.name) {
            let display = full[base.len().min(full.len())..]
                .trim_start_matches('\\')
                .into();
            out.push((display, host.native(&full)));
        }
    }
    for e in entries
        .iter()
        .filter(|e| e.meta.dir && e.meta.attrs & attr::HIDDEN == 0)
    {
        walk_files(host, &wpath::join(dir, &e.name), pattern, base, out);
    }
}
fn sort(cmd: &mut Cmd, host: &mut dyn Host, args: &[String]) -> Outcome {
    let mut reverse = false;
    let mut column = 0usize;
    let mut output: Option<String> = None;
    let mut file = None;
    let mut i = 0;
    while i < args.len() {
        let l = args[i].to_ascii_lowercase();
        if l == "/r" || l == "/reverse" {
            reverse = true;
        } else if let Some(n) = l.strip_prefix("/+") {
            column = n.parse::<usize>().unwrap_or(1).saturating_sub(1);
        } else if l == "/o" || l == "/output" {
            i += 1;
            output = args.get(i).map(|s| unq(s));
        } else if l.starts_with('/') {
        } else {
            file = Some(unq(&args[i]));
        }
        i += 1;
    }
    let text = match file {
        Some(f) => match cmd
            .native(host, &f)
            .ok()
            .and_then(|n| host.read(&n, 16 << 20).ok())
        {
            Some(b) => decode_text(&b),
            None => return err(cmd, host, "The system cannot find the file specified.", 1),
        },
        None => input_text(cmd).unwrap_or_default(),
    };
    let mut lines: Vec<&str> = text.lines().collect();
    lines.sort_by(|a, b| {
        let ka: String = a.chars().skip(column).collect::<String>().to_lowercase();
        let kb: String = b.chars().skip(column).collect::<String>().to_lowercase();
        ka.cmp(&kb)
    });
    if reverse {
        lines.reverse();
    }
    let mut out = lines.join("\n");
    if !out.is_empty() {
        out.push('\n');
    }
    match output {
        Some(o) => {
            let Ok(n) = cmd.native(host, &o) else {
                return err(cmd, host, "The system cannot find the path specified.", 1);
            };
            let crlf = out.replace('\n', "\r\n");
            match host.write(&n, crlf.as_bytes(), false) {
                Ok(()) => done(true),
                Err(e) => err(cmd, host, e.message(), 1),
            }
        }
        None => {
            cmd.out(host, &out);
            done(true)
        }
    }
}
fn more(cmd: &mut Cmd, host: &mut dyn Host, args: &[String]) -> Outcome {
    let names: Vec<String> = args
        .iter()
        .filter(|a| !a.starts_with('/') && !a.starts_with('+'))
        .cloned()
        .collect();
    let text = if names.is_empty() {
        input_text(cmd).unwrap_or_default()
    } else {
        let mut all = String::new();
        for (display, native) in file_list(cmd, host, &names) {
            match native.and_then(|n| host.read(&n, 16 << 20).ok()) {
                Some(b) => all.push_str(&decode_text(&b)),
                None => return err(cmd, host, &format!("Cannot access file {}", display), 1),
            }
        }
        all
    };
    let lines: Vec<String> = text.lines().map(String::from).collect();
    if !cmd.stdout_is_console() {
        for l in lines {
            cmd.out_line(host, &l);
        }
        return done(true);
    }
    let page = cmd.rows.saturating_sub(1).max(1);
    more_page(cmd, host, lines, 0, page)
}
fn more_page(
    cmd: &mut Cmd,
    host: &mut dyn Host,
    lines: Vec<String>,
    at: usize,
    count: usize,
) -> Outcome {
    let end = (at + count).min(lines.len());
    for l in &lines[at..end] {
        cmd.out_line(host, l);
    }
    if end >= lines.len() {
        return done(true);
    }
    let percent = end * 100 / lines.len().max(1);
    cmd.out(host, &format!("-- More ({percent}%) -- "));
    Outcome::Wait(
        BuiltinPending::Tool(Pending::More { lines, at: end }),
        Step::Input(InputKind::Key),
    )
}
/// Line diff blocks (FC's resynchronizing report, simplified to LCS hunks).
fn fc(cmd: &mut Cmd, host: &mut dyn Host, args: &[String]) -> Outcome {
    let mut ins = false;
    let mut numbers = false;
    let mut binary = false;
    let mut ws = false;
    let names: Vec<String> = args
        .iter()
        .filter(|a| {
            let l = a.to_ascii_lowercase();
            match l.as_str() {
                "/c" => ins = true,
                "/n" => numbers = true,
                "/b" => binary = true,
                "/w" => ws = true,
                _ => return !l.starts_with('/'),
            }
            false
        })
        .map(|a| unq(a))
        .collect();
    if names.len() != 2 {
        return err(
            cmd,
            host,
            "FC: Insufficient number of file specifications",
            2,
        );
    }
    let read = |cmd: &Cmd, host: &mut dyn Host, n: &str| {
        cmd.native(host, n)
            .ok()
            .and_then(|p| host.read(&p, 16 << 20).ok())
    };
    let (Some(a), Some(b)) = (read(cmd, host, &names[0]), read(cmd, host, &names[1])) else {
        let missing = if read(cmd, host, &names[0]).is_none() {
            &names[0]
        } else {
            &names[1]
        };
        return err(
            cmd,
            host,
            &format!(
                "FC: cannot open {} - No such file or folder",
                missing.to_uppercase()
            ),
            2,
        );
    };
    let second = names[1].to_uppercase();
    cmd.out_line(host, &format!("Comparing files {} and {second}", names[0]));
    if binary {
        let mut diffs = 0;
        for (i, (x, y)) in a.iter().zip(&b).enumerate() {
            if x != y {
                cmd.out_line(host, &format!("{i:08X}: {x:02X} {y:02X}"));
                diffs += 1;
            }
        }
        if a.len() != b.len() {
            let longer = if a.len() > b.len() {
                names[0].to_uppercase()
            } else {
                second.clone()
            };
            cmd.out_line(
                host,
                &format!(
                    "FC: {longer} longer than {}",
                    if a.len() > b.len() {
                        second.clone()
                    } else {
                        names[0].to_uppercase()
                    }
                ),
            );
            diffs += 1;
        }
        if diffs == 0 {
            cmd.out_line(host, "FC: no differences encountered\n");
        }
        cmd.errorlevel = i32::from(diffs > 0);
        return done(diffs == 0);
    }
    let ta = decode_text(&a);
    let tb = decode_text(&b);
    let la: Vec<&str> = ta.lines().collect();
    let lb: Vec<&str> = tb.lines().collect();
    let norm = |s: &str| {
        let s = if ws {
            s.split_whitespace().collect::<Vec<_>>().join(" ")
        } else {
            String::from(s)
        };
        if ins { s.to_lowercase() } else { s }
    };
    let (na, nb): (Vec<String>, Vec<String>) = (
        la.iter().map(|s| norm(s)).collect(),
        lb.iter().map(|s| norm(s)).collect(),
    );
    // LCS table (bounded sizes keep this cheap).
    if na.len() * nb.len() > 4_000_000 {
        return err(cmd, host, "FC: Resync failed.  Files are too different.", 1);
    }
    let mut l = vec![vec![0u32; nb.len() + 1]; na.len() + 1];
    for i in (0..na.len()).rev() {
        for j in (0..nb.len()).rev() {
            l[i][j] = if na[i] == nb[j] {
                l[i + 1][j + 1] + 1
            } else {
                l[i + 1][j].max(l[i][j + 1])
            };
        }
    }
    let (mut i, mut j) = (0, 0);
    let mut differences = false;
    let line = |n: usize, s: &str| {
        if numbers {
            format!("{:>5}:  {s}", n + 1)
        } else {
            String::from(s)
        }
    };
    while i < na.len() || j < nb.len() {
        if i < na.len() && j < nb.len() && na[i] == nb[j] {
            i += 1;
            j += 1;
            continue;
        }
        let (si, sj) = (i, j);
        while (i < na.len() || j < nb.len()) && !(i < na.len() && j < nb.len() && na[i] == nb[j]) {
            if j >= nb.len() || (i < na.len() && l[i + 1][j] >= l[i][j + 1]) {
                i += 1;
            } else {
                j += 1;
            }
        }
        differences = true;
        cmd.out_line(host, &format!("***** {}", names[0]));
        for (k, text) in la.iter().enumerate().take((i + 1).min(la.len())).skip(si) {
            cmd.out_line(host, &line(k, text));
        }
        cmd.out_line(host, &format!("***** {second}"));
        for (k, text) in lb.iter().enumerate().take((j + 1).min(lb.len())).skip(sj) {
            cmd.out_line(host, &line(k, text));
        }
        cmd.out_line(host, "*****\n");
    }
    if !differences {
        cmd.out_line(host, "FC: no differences encountered\n");
    }
    cmd.errorlevel = i32::from(differences);
    done(!differences)
}
fn comp(cmd: &mut Cmd, host: &mut dyn Host, args: &[String]) -> Outcome {
    let names: Vec<String> = args
        .iter()
        .filter(|a| !a.starts_with('/'))
        .map(|a| unq(a))
        .collect();
    let skip_prompt = args.iter().any(|a| a.eq_ignore_ascii_case("/m"));
    if names.len() < 2 {
        return err(
            cmd,
            host,
            "COMP: two file names are needed (COMP data1 data2).",
            1,
        );
    }
    let read = |cmd: &Cmd, host: &mut dyn Host, n: &str| {
        cmd.native(host, n)
            .ok()
            .and_then(|p| host.read(&p, 16 << 20).ok())
    };
    cmd.out_line(host, &format!("Comparing {} and {}...", names[0], names[1]));
    match (read(cmd, host, &names[0]), read(cmd, host, &names[1])) {
        (Some(a), Some(b)) if a.len() != b.len() => {
            cmd.out_line(host, "Files are different sizes.")
        }
        (Some(a), Some(b)) => {
            let mut errors = 0;
            for (i, (x, y)) in a.iter().zip(&b).enumerate() {
                if x != y && errors < 10 {
                    cmd.out_line(
                        host,
                        &format!("Compare error at OFFSET {i:X}\nfile1 = {x:X}\nfile2 = {y:X}"),
                    );
                    errors += 1;
                }
            }
            if errors == 0 {
                cmd.out_line(host, "Files compare OK");
            }
        }
        _ => cmd.out_line(host, "Can't find/open file"),
    }
    if skip_prompt || cmd.batch_depth() > 0 || cmd.input().is_some() {
        return done(true);
    }
    cmd.out(host, "\nCompare more files (Y/N) ? ");
    Outcome::Wait(
        BuiltinPending::Tool(Pending::Comp),
        Step::Input(InputKind::Key),
    )
}
fn tree(cmd: &mut Cmd, host: &mut dyn Host, args: &[String]) -> Outcome {
    let files = args.iter().any(|a| a.eq_ignore_ascii_case("/f"));
    let ascii = args.iter().any(|a| a.eq_ignore_ascii_case("/a"));
    let target = args.iter().find(|a| !a.starts_with('/')).map(|a| unq(a));
    let full = match &target {
        Some(t) => match cmd.full(t) {
            Ok(f) => f,
            Err(_) => return err(cmd, host, "Invalid path - \\", 1),
        },
        None => cmd.cwd.clone(),
    };
    let Some(native) = host
        .native(&full)
        .filter(|n| host.stat(n).is_ok_and(|m| m.dir))
    else {
        return err(
            cmd,
            host,
            &format!(
                "Invalid path - {}",
                target.unwrap_or_default().to_uppercase()
            ),
            1,
        );
    };
    let drive = wpath::drive(&full).unwrap_or('C');
    let v = host.volume(drive).unwrap_or_default();
    let label = if v.label.is_empty() {
        String::from("Folder PATH listing")
    } else {
        format!("Folder PATH listing for volume {}", v.label)
    };
    cmd.out_line(host, &label);
    cmd.out_line(
        host,
        &format!(
            "Volume serial number is {:04X}-{:04X}",
            v.serial >> 16,
            v.serial & 0xFFFF
        ),
    );
    let root = match &target {
        None => format!("{drive}:."),
        Some(_) => full.to_uppercase(),
    };
    cmd.out_line(host, &root);
    let (tee, last, bar, blank) = if ascii {
        ("+---", "\\---", "|   ", "    ")
    } else {
        ("├───", "└───", "│   ", "    ")
    };
    let mut any = false;
    tree_level(
        cmd,
        host,
        &native,
        "",
        files,
        (tee, last, bar, blank),
        &mut any,
    );
    if !any {
        cmd.out_line(host, "No subfolders exist \n");
    }
    done(true)
}
fn tree_level(
    cmd: &mut Cmd,
    host: &mut dyn Host,
    native: &str,
    prefix: &str,
    files: bool,
    g: (&str, &str, &str, &str),
    any: &mut bool,
) {
    let Ok(mut entries) = host.list(native) else {
        return;
    };
    entries.retain(|e| e.meta.attrs & (attr::HIDDEN | attr::SYSTEM) == 0);
    entries.sort_by_key(|e| e.name.to_lowercase());
    let dirs: Vec<&crate::host::Entry> = entries.iter().filter(|e| e.meta.dir).collect();
    if files {
        let fl: Vec<&crate::host::Entry> = entries.iter().filter(|e| !e.meta.dir).collect();
        let lead = if dirs.is_empty() { g.3 } else { g.2 };
        for f in &fl {
            cmd.out_line(host, &format!("{prefix}{lead}{}", f.name));
        }
        if !fl.is_empty() {
            cmd.out_line(host, &format!("{prefix}{lead}"));
        }
    }
    for (i, d) in dirs.iter().enumerate() {
        *any = true;
        let is_last = i + 1 == dirs.len();
        cmd.out_line(
            host,
            &format!("{prefix}{}{}", if is_last { g.1 } else { g.0 }, d.name),
        );
        let child = format!("{}/{}", native.trim_end_matches('/'), d.name);
        let next = format!("{prefix}{}", if is_last { g.3 } else { g.2 });
        tree_level(cmd, host, &child, &next, files, g, any);
    }
}
fn attrib(cmd: &mut Cmd, host: &mut dyn Host, args: &[String]) -> Outcome {
    let mut set = 0u32;
    let mut clear = 0u32;
    let mut subdirs = false;
    let mut dirs = false;
    let mut names = Vec::new();
    for a in args {
        let l = a.to_ascii_lowercase();
        let bit = |c: char| match c {
            'r' => attr::READONLY,
            'a' => attr::ARCHIVE,
            's' => attr::SYSTEM,
            'h' => attr::HIDDEN,
            'i' => attr::NOT_CONTENT_INDEXED,
            'o' => attr::OFFLINE,
            _ => 0,
        };
        if let Some(c) = l.strip_prefix('+') {
            set |= c.chars().map(bit).fold(0, |a, b| a | b);
        } else if let Some(c) = l.strip_prefix('-') {
            clear |= c.chars().map(bit).fold(0, |a, b| a | b);
        } else if l == "/s" {
            subdirs = true;
        } else if l == "/d" {
            dirs = true;
        } else if l == "/l" {
        } else if l.starts_with('/') {
            return err(
                cmd,
                host,
                &format!("Parameter format not correct - \"{a}\"."),
                1,
            );
        } else {
            names.push(unq(a));
        }
    }
    if names.is_empty() {
        names.push(String::from("*"));
    }
    let mut targets: Vec<(String, Meta)> = Vec::new();
    for n in &names {
        let Ok(full) = cmd.full(n) else { continue };
        let (dir, pat) = wpath::split(&full);
        let (dir, pat) = (String::from(dir), String::from(pat));
        collect_all(
            host,
            &dir,
            &pat,
            subdirs,
            dirs || (set | clear) != 0 && !wpath::wild(&pat),
            &mut targets,
        );
    }
    if targets.is_empty() {
        let name = names[0].clone();
        let full = cmd.full(&name).unwrap_or(name);
        return err(cmd, host, &format!("File not found - {full}"), 1);
    }
    for (path, m) in targets {
        if set | clear != 0 {
            let Some(n) = host.native(&path) else {
                continue;
            };
            let attrs = (m.attrs | set) & !clear;
            if let Err(e) = host.set_attributes(&n, attrs) {
                cmd.err_line(
                    host,
                    &format!("{} - {path}", e.message().trim_end_matches('.')),
                );
            }
            continue;
        }
        let flag = |b: u32, c: char| if m.attrs & b != 0 { c } else { ' ' };
        let mut col = [' '; 21];
        col[0] = flag(attr::ARCHIVE, 'A');
        col[3] = flag(attr::SYSTEM, 'S');
        col[4] = flag(attr::HIDDEN, 'H');
        col[5] = flag(attr::READONLY, 'R');
        col[6] = flag(attr::OFFLINE, 'O');
        col[8] = flag(attr::NOT_CONTENT_INDEXED, 'I');
        let flags: String = col.iter().collect();
        cmd.out_line(host, &format!("{flags}{path}"));
    }
    done(true)
}
fn collect_all(
    host: &mut dyn Host,
    dir: &str,
    pattern: &str,
    subdirs: bool,
    dirs: bool,
    out: &mut Vec<(String, Meta)>,
) {
    let Some(native) = host.native(dir) else {
        return;
    };
    let Ok(mut entries) = host.list(&native) else {
        return;
    };
    entries.sort_by_key(|e| e.name.to_lowercase());
    for e in &entries {
        if (!e.meta.dir || dirs) && wild::matches(pattern, &e.name) {
            out.push((wpath::join(dir, &e.name), e.meta));
        }
    }
    if subdirs {
        for e in entries.iter().filter(|e| e.meta.dir) {
            collect_all(host, &wpath::join(dir, &e.name), pattern, true, dirs, out);
        }
    }
}
fn xcopy(cmd: &mut Cmd, host: &mut dyn Host, args: &[String]) -> Outcome {
    let mut sw = String::new();
    let mut names = Vec::new();
    for a in args {
        if let Some(s) = a.strip_prefix('/') {
            sw.push_str(&s.to_ascii_lowercase());
            sw.push(' ');
        } else {
            names.push(unq(a));
        }
    }
    let has = |f: &str| sw.split(' ').any(|s| s == f);
    let (subdirs, empty, quiet, full_names, list, hidden, tree_only) = (
        has("s") || has("e"),
        has("e"),
        has("q"),
        has("f"),
        has("l"),
        has("h"),
        has("t"),
    );
    let Some(src) = names.first() else {
        return err(cmd, host, "Invalid number of parameters", 4);
    };
    let Ok(src_full) = cmd.full(src) else {
        return err(cmd, host, "Invalid path", 4);
    };
    let dest_full = match names.get(1) {
        Some(d) => match cmd.full(d) {
            Ok(f) => f,
            Err(_) => return err(cmd, host, "Invalid path", 4),
        },
        None => cmd.cwd.clone(),
    };
    let (src_dir, pattern) = match host.native(&src_full).map(|n| host.stat(&n)) {
        Some(Ok(m)) if m.dir => (src_full.clone(), String::from("*")),
        Some(Ok(_)) | None | Some(Err(_)) => {
            let (d, p) = wpath::split(&src_full);
            (String::from(d), String::from(p))
        }
    };
    if host.native(&src_dir).is_none_or(|n| host.stat(&n).is_err()) {
        return err(
            cmd,
            host,
            &format!("File not found - {}", wpath::name(&src_full)),
            4,
        );
    }
    let mut copied = 0usize;
    let mut plan: Vec<(String, String, bool)> = Vec::new();
    xcopy_walk(
        host, &src_dir, &dest_full, &pattern, subdirs, empty, hidden, &mut plan,
    );
    for (from, to, dir) in plan {
        let (Some(a), Some(b)) = (host.native(&from), host.native(&to)) else {
            continue;
        };
        if dir {
            if !list {
                let _ = host.mkdir(&b);
            }
            continue;
        }
        if tree_only {
            continue;
        }
        // Parent folders on the destination side.
        let parent = wpath::parent(&to);
        if let Some(pn) = host.native(parent)
            && host.stat(&pn).is_err()
            && !list
        {
            let mut chain = Vec::new();
            let mut cur = String::from(parent);
            while host.native(&cur).is_some_and(|n| host.stat(&n).is_err()) && !wpath::is_root(&cur)
            {
                chain.push(cur.clone());
                cur = String::from(wpath::parent(&cur));
            }
            for c in chain.iter().rev() {
                if let Some(n) = host.native(c) {
                    let _ = host.mkdir(&n);
                }
            }
        }
        let display = if full_names {
            format!("{from} -> {to}")
        } else {
            from[src_dir.len().min(from.len())..]
                .trim_start_matches('\\')
                .into()
        };
        let shown = if full_names {
            display
        } else {
            format!("{}\\{display}", wpath::name(&src_dir))
        };
        if list || host.copy_file(&a, &b).is_ok() {
            if !quiet {
                cmd.out_line(host, &shown);
            }
            copied += 1;
        }
    }
    if list {
        cmd.out_line(host, &format!("{copied} File(s)"));
    } else {
        cmd.out_line(host, &format!("{copied} File(s) copied"));
    }
    cmd.errorlevel = if copied == 0 { 1 } else { 0 };
    done(true)
}
#[allow(clippy::too_many_arguments)]
fn xcopy_walk(
    host: &mut dyn Host,
    src: &str,
    dest: &str,
    pattern: &str,
    subdirs: bool,
    empty: bool,
    hidden: bool,
    plan: &mut Vec<(String, String, bool)>,
) {
    let Some(native) = host.native(src) else {
        return;
    };
    let Ok(mut entries) = host.list(&native) else {
        return;
    };
    entries.sort_by_key(|e| e.name.to_lowercase());
    let visible = |m: &Meta| hidden || m.attrs & (attr::HIDDEN | attr::SYSTEM) == 0;
    for e in entries
        .iter()
        .filter(|e| !e.meta.dir && visible(&e.meta) && wild::matches(pattern, &e.name))
    {
        plan.push((wpath::join(src, &e.name), wpath::join(dest, &e.name), false));
    }
    if subdirs {
        for e in entries.iter().filter(|e| e.meta.dir && visible(&e.meta)) {
            let (s, d) = (wpath::join(src, &e.name), wpath::join(dest, &e.name));
            let before = plan.len();
            if empty {
                plan.push((s.clone(), d.clone(), true));
            }
            xcopy_walk(host, &s, &d, pattern, true, empty, hidden, plan);
            let _ = before;
        }
    }
}
fn robocopy(cmd: &mut Cmd, host: &mut dyn Host, args: &[String]) -> Outcome {
    let plain: Vec<String> = args
        .iter()
        .filter(|a| !a.starts_with('/'))
        .map(|a| unq(a))
        .collect();
    let sw: Vec<String> = args
        .iter()
        .filter(|a| a.starts_with('/'))
        .map(|a| a.to_ascii_lowercase())
        .collect();
    if plain.len() < 2 {
        cmd.out_line(host, "\n-------------------------------------------------------------------------------\n   ROBOCOPY     ::     Robust File Copy for Windows\n-------------------------------------------------------------------------------\n\n  Usage :: ROBOCOPY source destination [file [file]...] [options]");
        return Outcome::Done(false);
    }
    let (Ok(src), Ok(dst)) = (cmd.full(&plain[0]), cmd.full(&plain[1])) else {
        return err(cmd, host, "ERROR : Invalid Parameter #1", 16);
    };
    let patterns: Vec<String> = if plain.len() > 2 {
        plain[2..].to_vec()
    } else {
        vec![String::from("*.*")]
    };
    let recurse = sw.iter().any(|s| s == "/s" || s == "/e" || s == "/mir");
    let empty = sw.iter().any(|s| s == "/e" || s == "/mir");
    let list = sw.iter().any(|s| s == "/l");
    let now = host.now();
    cmd.out_line(host, &format!(
        "\n-------------------------------------------------------------------------------\n   ROBOCOPY     ::     Robust File Copy for Windows                              \n-------------------------------------------------------------------------------\n\n  Started : {}\n   Source : {src}\\\n     Dest : {dst}\\\n\n    Files : {}\n\n  Options : /DCOPY:DA /COPY:DAT /R:1000000 /W:30 \n\n------------------------------------------------------------------------------\n",
        date_time_full(&now),
        patterns.join(" ")
    ));
    let mut plan = Vec::new();
    for p in &patterns {
        xcopy_walk(host, &src, &dst, p, recurse, empty, false, &mut plan);
    }
    let (mut files, mut bytes, mut dirs) = (0u64, 0u64, 1u64);
    if let Some(n) = host.native(&dst)
        && !list
    {
        let _ = host.mkdir(&n);
    }
    for (from, to, dir) in plan {
        let (Some(a), Some(b)) = (host.native(&from), host.native(&to)) else {
            continue;
        };
        if dir {
            dirs += 1;
            if !list {
                let _ = host.mkdir(&b);
            }
            continue;
        }
        if let Some(pn) = host.native(wpath::parent(&to))
            && host.stat(&pn).is_err()
            && !list
        {
            let _ = host.mkdir(&pn);
        }
        let size = host.stat(&a).map(|m| m.size).unwrap_or(0);
        if list || host.copy_file(&a, &b).is_ok() {
            files += 1;
            bytes += size;
            cmd.out_line(
                host,
                &format!("\t    New File  \t\t{:>8}\t{}", size, wpath::name(&from)),
            );
        }
    }
    let end = host.now();
    cmd.out_line(host, &format!(
        "\n------------------------------------------------------------------------------\n\n               Total    Copied   Skipped  Mismatch    FAILED    Extras\n    Dirs : {dirs:>9} {dirs:>9}         0         0         0         0\n   Files : {files:>9} {files:>9}         0         0         0         0\n   Bytes : {bytes:>9} {bytes:>9}         0         0         0         0\n\n   Ended : {}\n",
        date_time_full(&end)
    ));
    // Robocopy's exit code: 1 when files were copied.
    cmd.errorlevel = i32::from(files > 0);
    done(true)
}
fn where_(cmd: &mut Cmd, host: &mut dyn Host, args: &[String]) -> Outcome {
    let quiet = args.iter().any(|a| a.eq_ignore_ascii_case("/q"));
    let quoted = args.iter().any(|a| a.eq_ignore_ascii_case("/f"));
    let mut recurse_dir: Option<String> = None;
    let mut patterns = Vec::new();
    let mut i = 0;
    while i < args.len() {
        let l = args[i].to_ascii_lowercase();
        if l == "/r" {
            i += 1;
            recurse_dir = args.get(i).map(|s| unq(s));
        } else if !l.starts_with('/') {
            patterns.push(unq(&args[i]));
        }
        i += 1;
    }
    if patterns.is_empty() {
        return err(cmd, host, "ERROR: A search pattern must be specified.", 2);
    }
    let pathext: Vec<String> = cmd
        .env
        .get("PATHEXT")
        .unwrap_or(".COM;.EXE;.BAT;.CMD")
        .split(';')
        .map(|e| e.to_ascii_lowercase())
        .collect();
    let mut dirs = vec![cmd.cwd.clone()];
    let path_var = cmd.env.get("PATH").map(String::from).unwrap_or_default();
    dirs.extend(
        path_var
            .split(';')
            .filter(|d| !d.is_empty())
            .map(|d| String::from(d.trim_end_matches('\\'))),
    );
    let mut found = Vec::new();
    for p in &patterns {
        let has_ext = !wpath::extension(p).is_empty() || p.contains('*');
        let names: Vec<String> = if has_ext {
            vec![p.clone()]
        } else {
            core::iter::once(p.clone())
                .chain(pathext.iter().map(|e| format!("{p}{e}")))
                .collect()
        };
        let search: Vec<String> = match &recurse_dir {
            Some(r) => {
                let root = cmd.full(r).unwrap_or_else(|_| cmd.cwd.clone());
                let mut all = Vec::new();
                tree_dirs(host, &root, &mut all, 0);
                all
            }
            None => dirs.clone(),
        };
        for d in &search {
            let Some(native) = host.native(d) else {
                continue;
            };
            let Ok(entries) = host.list(&native) else {
                continue;
            };
            for n in &names {
                for e in entries
                    .iter()
                    .filter(|e| !e.meta.dir && wild::matches(n, &e.name))
                {
                    let full = wpath::join(d, &e.name);
                    if !found.contains(&full) {
                        found.push(full);
                    }
                }
            }
            // The built-in System32 tools.
            if d.eq_ignore_ascii_case("C:\\Windows\\system32") {
                let stem = p.trim_end_matches(".exe").to_ascii_lowercase();
                if NAMES.contains(&stem.as_str()) {
                    let full = format!("C:\\Windows\\System32\\{stem}.exe");
                    if !found.iter().any(|f| f.eq_ignore_ascii_case(&full)) {
                        found.push(full);
                    }
                }
                if stem == "cmd"
                    && !found
                        .iter()
                        .any(|f| f.to_ascii_lowercase().ends_with("\\cmd.exe"))
                {
                    found.push(String::from("C:\\Windows\\System32\\cmd.exe"));
                }
            }
        }
    }
    if found.is_empty() {
        if !quiet {
            cmd.err_line(host, "INFO: Could not find files for the given pattern(s).");
        }
        cmd.errorlevel = 1;
        return Outcome::Done(false);
    }
    if !quiet {
        for f in found {
            let line = if quoted { format!("\"{f}\"") } else { f };
            cmd.out_line(host, &line);
        }
    }
    done(true)
}
fn tree_dirs(host: &mut dyn Host, dir: &str, out: &mut Vec<String>, depth: usize) {
    out.push(String::from(dir));
    if depth > 32 {
        return;
    }
    let Some(native) = host.native(dir) else {
        return;
    };
    let Ok(entries) = host.list(&native) else {
        return;
    };
    for e in entries.iter().filter(|e| e.meta.dir) {
        tree_dirs(host, &wpath::join(dir, &e.name), out, depth + 1);
    }
}
fn whoami(cmd: &mut Cmd, host: &mut dyn Host, args: &[String]) -> Outcome {
    let name = format!("{}\\{}", host.domain(), host.user_name()).to_lowercase();
    let sid = host.user_sid();
    let opt = args
        .first()
        .map(|a| a.to_ascii_lowercase())
        .unwrap_or_default();
    match opt.as_str() {
        "" => cmd.out_line(host, &name),
        "/user" | "/all" | "/groups" | "/priv" => {
            if opt == "/user" || opt == "/all" {
                let w = name.len().max(9);
                cmd.out_line(
                    host,
                    &format!(
                        "\nUSER INFORMATION\n----------------\n\n{:<w$} SID\n{} {}\n{:<w$} {sid}",
                        "User Name",
                        "=".repeat(w),
                        "=".repeat(sid.len().max(3)),
                        name
                    ),
                );
            }
            if opt == "/groups" || opt == "/all" {
                // An administrator's unelevated token keeps Administrators
                // for deny only at Medium integrity; elevated, it is enabled
                // at High integrity.
                let (admins, label) = if host.elevated() {
                    (
                        "Mandatory group, Enabled by default, Enabled group, Group owner",
                        "Mandatory Label\\High Mandatory Level   Label            S-1-16-12288",
                    )
                } else {
                    (
                        "Group used for deny only",
                        "Mandatory Label\\Medium Mandatory Level Label            S-1-16-8192",
                    )
                };
                // A standard user's token has no Administrators group.
                let admins = if host.elevated() || host.administrator() {
                    format!(
                        "BUILTIN\\Administrators                 Alias            S-1-5-32-544 {admins}\n"
                    )
                } else {
                    String::new()
                };
                cmd.out_line(host, &format!("\n\nGROUP INFORMATION\n-----------------\n\nGroup Name                             Type             SID          Attributes\n====================================== ================ ============ ==================================================\nEveryone                               Well-known group S-1-1-0      Mandatory group, Enabled by default, Enabled group\n{admins}BUILTIN\\Users                          Alias            S-1-5-32-545 Mandatory group, Enabled by default, Enabled group\nNT AUTHORITY\\INTERACTIVE               Well-known group S-1-5-4      Mandatory group, Enabled by default, Enabled group\nNT AUTHORITY\\Authenticated Users       Well-known group S-1-5-11     Mandatory group, Enabled by default, Enabled group\n{label}"));
            }
            if opt == "/priv" || opt == "/all" {
                cmd.out_line(host, "\n\nPRIVILEGES INFORMATION\n----------------------\n\nPrivilege Name                Description                          State\n============================= ==================================== ========\nSeShutdownPrivilege           Shut down the system                 Disabled\nSeChangeNotifyPrivilege       Bypass traverse checking             Enabled\nSeUndockPrivilege             Remove computer from docking station Disabled\nSeIncreaseWorkingSetPrivilege Increase a process working set       Disabled\nSeTimeZonePrivilege           Change the time zone                 Disabled");
            }
        }
        "/upn" | "/fqdn" => {
            return err(
                cmd,
                host,
                "ERROR: Unable to get User Principal Name (UPN) as the current logged-on user\n       is not a domain user.",
                1,
            );
        }
        "/logonid" => cmd.out_line(host, "S-1-5-5-0-1001"),
        _ => {
            return err(
                cmd,
                host,
                &format!(
                    "ERROR: Invalid argument/option - '{}'.\nType \"WHOAMI /?\" for usage.",
                    args[0]
                ),
                1,
            );
        }
    }
    done(true)
}
fn ip4(a: [u8; 4]) -> String {
    format!("{}.{}.{}.{}", a[0], a[1], a[2], a[3])
}
fn mask4(m: [u8; 4]) -> String {
    ip4(m)
}
fn mac(m: [u8; 6], sep: char) -> String {
    let mut s = String::new();
    for (i, b) in m.iter().enumerate() {
        if i > 0 {
            s.push(sep);
        }
        s.push_str(&format!("{b:02X}"));
    }
    s
}
fn ipconfig(cmd: &mut Cmd, host: &mut dyn Host, args: &[String]) -> Outcome {
    let opt = args
        .first()
        .map(|a| a.to_ascii_lowercase())
        .unwrap_or_default();
    match opt.as_str() {
        "/flushdns" => {
            cmd.out_line(
                host,
                "\nDoors IP Configuration\n\nSuccessfully flushed the DNS Resolver Cache.",
            );
            return done(true);
        }
        "/displaydns" => {
            cmd.out_line(
                host,
                "\nDoors IP Configuration\n\nCould not display the DNS Resolver Cache.",
            );
            return Outcome::Done(false);
        }
        "/release" | "/renew" | "/registerdns" => {
            return err(
                cmd,
                host,
                "\nDoors IP Configuration\n\nThe requested operation requires elevation.",
                1,
            );
        }
        "" | "/all" => (),
        _ => {
            return err(
                cmd,
                host,
                &format!(
                    "Error: unrecognized or incomplete command line.\n\nUSAGE:\n    ipconfig [/all | /release | /renew | /flushdns | /displaydns]\n\n(option {})",
                    args[0]
                ),
                1,
            );
        }
    }
    let all = opt == "/all";
    cmd.out_line(host, "\nDoors IP Configuration\n");
    if all {
        let name = host.computer_name();
        cmd.out_line(host, &format!("   Host Name . . . . . . . . . . . . : {name}\n   Primary Dns Suffix  . . . . . . . : \n   Node Type . . . . . . . . . . . . : Hybrid\n   IP Routing Enabled. . . . . . . . : No\n   WINS Proxy Enabled. . . . . . . . : No\n"));
    }
    for a in host.adapters() {
        // A VPN tunnel is a virtual adapter Windows names no kind for.
        let kind = if a.tunnel { "Unknown" } else { "Ethernet" };
        cmd.out_line(host, &format!("{kind} adapter {}:\n", a.name));
        if !a.connected || a.ipv4.is_none() {
            cmd.out_line(host, &format!("   Media State . . . . . . . . . . . : Media disconnected\n   Connection-specific DNS Suffix  . : {}\n", a.suffix));
            continue;
        }
        let ip = ip4(a.ipv4.unwrap());
        cmd.out_line(
            host,
            &format!("   Connection-specific DNS Suffix  . : {}", a.suffix),
        );
        let preferred = if all { "(Preferred) " } else { "" };
        let v6 = |b: [u8; 16]| format!("{}", core::net::Ipv6Addr::from(b));
        let scoped = |b: [u8; 16]| format!("{}%{}", core::net::Ipv6Addr::from(b), a.scope);
        // The IPv6 router heads the gateway list, as Windows prints it.
        let gateways: Vec<String> = a
            .gateway6
            .map(scoped)
            .into_iter()
            .chain(a.gateway.map(ip4))
            .collect();
        let mut gateway_text = format!(
            "   Default Gateway . . . . . . . . . : {}",
            gateways.first().cloned().unwrap_or_default()
        );
        for g in gateways.iter().skip(1) {
            gateway_text.push_str(&format!("\n                                       {g}"));
        }
        if all {
            cmd.out_line(host, &format!("   Description . . . . . . . . . . . : {}\n   Physical Address. . . . . . . . . : {}\n   DHCP Enabled. . . . . . . . . . . : {}\n   Autoconfiguration Enabled . . . . : Yes", a.description, if a.tunnel { String::new() } else { mac(a.mac, '-') }, if a.dhcp { "Yes" } else { "No" }));
        }
        if let Some(g) = a.ipv6 {
            cmd.out_line(
                host,
                &format!(
                    "   IPv6 Address. . . . . . . . . . . : {}{preferred}",
                    v6(g)
                ),
            );
        }
        if let Some(l) = a.link_local {
            cmd.out_line(
                host,
                &format!(
                    "   Link-local IPv6 Address . . . . . : {}{preferred}",
                    scoped(l)
                ),
            );
        }
        if all {
            cmd.out_line(host, &format!("   IPv4 Address. . . . . . . . . . . : {ip}(Preferred) \n   Subnet Mask . . . . . . . . . . . : {}", mask4(a.mask)));
            if let Some(t) = a.lease_obtained {
                cmd.out_line(
                    host,
                    &format!(
                        "   Lease Obtained. . . . . . . . . . : {}",
                        date_time_full(&t)
                    ),
                );
            }
            if let Some(t) = a.lease_expires {
                cmd.out_line(
                    host,
                    &format!(
                        "   Lease Expires . . . . . . . . . . : {}",
                        date_time_full(&t)
                    ),
                );
            }
            cmd.out_line(host, &gateway_text);
            if let Some(d) = a.dhcp_server {
                cmd.out_line(
                    host,
                    &format!("   DHCP Server . . . . . . . . . . . : {}", ip4(d)),
                );
            }
            let servers: Vec<String> = a
                .dns
                .iter()
                .map(|d| ip4(*d))
                .chain(a.dns6.iter().map(|d| v6(*d)))
                .collect();
            for (i, d) in servers.iter().enumerate() {
                if i == 0 {
                    cmd.out_line(host, &format!("   DNS Servers . . . . . . . . . . . : {d}"));
                } else {
                    cmd.out_line(host, &format!("                                       {d}"));
                }
            }
            cmd.out_line(host, "   NetBIOS over Tcpip. . . . . . . . : Enabled\n");
        } else {
            cmd.out_line(host, &format!("   IPv4 Address. . . . . . . . . . . : {ip}\n   Subnet Mask . . . . . . . . . . . : {}\n{gateway_text}\n", mask4(a.mask)));
        }
    }
    done(true)
}
fn getmac(cmd: &mut Cmd, host: &mut dyn Host) -> Outcome {
    cmd.out_line(host, "\nPhysical Address    Transport Name\n=================== ==========================================================");
    // A tunnel has no physical address: getmac leaves it out.
    for a in host.adapters().into_iter().filter(|a| !a.tunnel) {
        let t = if a.connected {
            String::from("\\Device\\Tcpip_{DOORS-ETHERNET}")
        } else {
            String::from("Media disconnected")
        };
        cmd.out_line(host, &format!("{:<20}{t}", mac(a.mac, '-')));
    }
    done(true)
}
fn nslookup(cmd: &mut Cmd, host: &mut dyn Host, args: &[String]) -> Outcome {
    let dns = host.adapters().into_iter().flat_map(|a| a.dns).next();
    let server = dns.map(ip4).unwrap_or_else(|| String::from("Unknown"));
    cmd.out_line(host, &format!("Server:  UnKnown\nAddress:  {server}\n"));
    let Some(name) = args.iter().find(|a| !a.starts_with('-')) else {
        return done(true);
    };
    match host.resolve(name) {
        Some(a) => {
            cmd.out_line(
                host,
                &format!(
                    "Non-authoritative answer:\nName:    {name}\nAddress:  {}\n",
                    ip4(a)
                ),
            );
            done(true)
        }
        None => err(
            cmd,
            host,
            &format!("*** UnKnown can't find {name}: Non-existent domain"),
            1,
        ),
    }
}
fn ping(cmd: &mut Cmd, host: &mut dyn Host, args: &[String]) -> Outcome {
    let mut count = Some(4u32);
    let mut bytes = 32usize;
    let mut timeout = 4000u64;
    let mut ttl = 128u8;
    let mut target = None;
    let mut i = 0;
    while i < args.len() {
        let l = args[i].to_ascii_lowercase();
        let next = |i: &mut usize| {
            *i += 1;
            args.get(*i).and_then(|v| v.parse::<u64>().ok())
        };
        match l.as_str() {
            "-n" | "/n" => count = next(&mut i).map(|v| v as u32),
            "-l" | "/l" => bytes = next(&mut i).unwrap_or(32) as usize,
            "-w" | "/w" => timeout = next(&mut i).unwrap_or(4000),
            "-i" | "/i" => ttl = next(&mut i).unwrap_or(128).clamp(1, 255) as u8,
            "-t" | "/t" => count = None,
            "-4" | "/4" | "-a" | "/a" => (),
            _ if l.starts_with('-') || l.starts_with('/') => (),
            _ => target = Some(args[i].clone()),
        }
        i += 1;
    }
    let Some(name) = target else {
        cmd.out(host, &super::help::text("ping"));
        return Outcome::Done(false);
    };
    let Some(address) = host.resolve(&name) else {
        return err(
            cmd,
            host,
            &format!(
                "Ping request could not find host {name}. Please check the name and try again."
            ),
            1,
        );
    };
    let ip = ip4(address);
    let shown = if ip == name {
        ip.clone()
    } else {
        format!("{name} [{ip}]")
    };
    cmd.out_line(
        host,
        &format!("\nPinging {shown} with {bytes} bytes of data:"),
    );
    ping_next(
        cmd,
        host,
        PingState {
            name,
            address,
            bytes,
            count,
            sent: 0,
            received: 0,
            times: Vec::new(),
            timeout,
            ttl,
        },
    )
}
fn ping_next(cmd: &mut Cmd, host: &mut dyn Host, mut s: PingState) -> Outcome {
    if s.count.is_some_and(|c| s.sent >= c) {
        let ip = ip4(s.address);
        let lost = s.sent - s.received;
        cmd.out_line(host, &format!("\nPing statistics for {ip}:\n    Packets: Sent = {}, Received = {}, Lost = {} ({}% loss),", s.sent, s.received, lost, lost * 100 / s.sent.max(1)));
        if !s.times.is_empty() {
            let min = s.times.iter().min().unwrap();
            let max = s.times.iter().max().unwrap();
            let avg = s.times.iter().sum::<u64>() / s.times.len() as u64;
            cmd.out_line(host, &format!("Approximate round trip times in milli-seconds:\n    Minimum = {min}ms, Maximum = {max}ms, Average = {avg}ms"));
        }
        let _ = &s.name;
        // ping's exit code is 1 only when no echo reply came back.
        cmd.errorlevel = i32::from(s.times.is_empty());
        return done(!s.times.is_empty());
    }
    s.sent += 1;
    match host.echo(s.address, s.bytes, s.timeout, s.ttl) {
        Ok(Echo::Reply {
            from,
            ms,
            ttl,
            bytes,
        }) => {
            s.received += 1;
            s.times.push(ms);
            let t = if ms == 0 {
                String::from("time<1ms")
            } else {
                format!("time={ms}ms")
            };
            cmd.out_line(
                host,
                &format!("Reply from {}: bytes={bytes} {t} TTL={ttl}", ip4(from)),
            );
        }
        Ok(Echo::TimedOut) => cmd.out_line(host, "Request timed out."),
        // Windows counts these as received (they are replies).
        Ok(Echo::TtlExpired { from, .. }) => {
            s.received += 1;
            cmd.out_line(
                host,
                &format!("Reply from {}: TTL expired in transit.", ip4(from)),
            );
        }
        Ok(Echo::Unreachable { from, status }) => {
            s.received += 1;
            let what = match status {
                11002 => "Destination net unreachable.",
                11004 => "Destination protocol unreachable.",
                11005 => "Destination port unreachable.",
                11009 => "Packet needs to be fragmented but DF set.",
                _ => "Destination host unreachable.",
            };
            cmd.out_line(host, &format!("Reply from {}: {what}", ip4(from)));
        }
        Ok(Echo::Failed) => cmd.out_line(host, "General failure."),
        Err(_) => cmd.out_line(host, "PING: transmit failed. General failure. "),
    }
    if s.count.is_some_and(|c| s.sent >= c) {
        return ping_next(cmd, host, s);
    }
    Outcome::Wait(
        BuiltinPending::Tool(Pending::Ping(s)),
        Step::Sleep {
            ms: 1000,
            keys: false,
        },
    )
}
/// tracert: three echoes per hop with TTL 1, 2, ... until the target
/// answers or `max` hops.
pub(crate) struct TraceState {
    address: [u8; 4],
    hop: u8,
    max: u8,
    timeout: u64,
}
fn tracert(cmd: &mut Cmd, host: &mut dyn Host, args: &[String]) -> Outcome {
    let mut max = 30u8;
    let mut timeout = 4000u64;
    let mut target = None;
    let mut i = 0;
    while i < args.len() {
        let l = args[i].to_ascii_lowercase();
        let mut next = || {
            i += 1;
            args.get(i).and_then(|v| v.parse::<u64>().ok())
        };
        match l.as_str() {
            "-h" | "/h" => max = next().unwrap_or(30).clamp(1, 255) as u8,
            "-w" | "/w" => timeout = next().unwrap_or(4000).max(1),
            "-j" | "/j" => {
                next();
            }
            "-d" | "/d" | "-4" | "/4" => (),
            _ if l.starts_with('-') || l.starts_with('/') => (),
            _ => target = Some(args[i].clone()),
        }
        i += 1;
    }
    let Some(name) = target else {
        cmd.out(host, &super::help::text("tracert"));
        return Outcome::Done(false);
    };
    let Some(address) = host.resolve(&name) else {
        return err(
            cmd,
            host,
            &format!("Unable to resolve target system name {name}."),
            1,
        );
    };
    let ip = ip4(address);
    let shown = if ip == name {
        ip.clone()
    } else {
        format!("{name} [{ip}]")
    };
    cmd.out_line(
        host,
        &format!("\nTracing route to {shown}\nover a maximum of {max} hops:\n"),
    );
    trace_next(
        cmd,
        host,
        TraceState {
            address,
            hop: 1,
            max,
            timeout,
        },
    )
}
fn trace_next(cmd: &mut Cmd, host: &mut dyn Host, mut s: TraceState) -> Outcome {
    let mut fields = String::new();
    let mut from = None;
    let mut reached = false;
    for _ in 0..3 {
        match host.echo(s.address, 32, s.timeout, s.hop) {
            Ok(Echo::Reply { from: f, ms, .. }) => {
                fields.push_str(&trace_time(ms));
                from = Some(f);
                reached = true;
            }
            Ok(Echo::TtlExpired { from: f, ms }) => {
                fields.push_str(&trace_time(ms));
                from = Some(f);
            }
            Ok(Echo::Unreachable { from: f, .. }) => {
                fields.push_str("     *   ");
                from = Some(f);
                reached = true;
            }
            _ => fields.push_str("     *   "),
        }
    }
    let line = match from {
        Some(f) => format!("{:>3}{fields}  {}", s.hop, ip4(f)),
        None => format!("{:>3}{fields}  Request timed out.", s.hop),
    };
    cmd.out_line(host, &line);
    if reached || s.hop >= s.max {
        cmd.out_line(host, "\nTrace complete.");
        return done(true);
    }
    s.hop += 1;
    Outcome::Wait(
        BuiltinPending::Tool(Pending::Trace(s)),
        Step::Sleep { ms: 0, keys: false },
    )
}
/// One tracert probe column: "    <1 ms", "    12 ms".
fn trace_time(ms: u64) -> String {
    let t = if ms == 0 {
        String::from("<1")
    } else {
        format!("{ms}")
    };
    format!("{t:>6} ms")
}
/// netstat [-a] [-n] [-o] [-p proto]: the stack's TCP connections (and,
/// with -a, listeners and UDP endpoints).
/// `arp -a` / `arp -g` (optionally for one address, or `-N` one interface):
/// the Ethernet adapter's neighbours, as Windows prints them.
fn arp(cmd: &mut Cmd, host: &mut dyn Host, args: &[String]) -> Outcome {
    let lower: Vec<String> = args.iter().map(|a| a.to_ascii_lowercase()).collect();
    let mut wanted: Option<[u8; 4]> = None;
    let mut interface: Option<[u8; 4]> = None;
    let mut i = 0;
    let mut listing = false;
    while i < lower.len() {
        match lower[i].as_str() {
            "-a" | "/a" | "-g" | "/g" | "-v" | "/v" => listing = true,
            "-n" | "/n" => {
                i += 1;
                interface = lower.get(i).and_then(|a| parse_ip4(a));
                if interface.is_none() {
                    cmd.out(host, &super::help::text("arp"));
                    return Outcome::Done(false);
                }
            }
            other if listing && wanted.is_none() && parse_ip4(other).is_some() => {
                wanted = parse_ip4(other);
            }
            _ => {
                cmd.out(host, &super::help::text("arp"));
                return Outcome::Done(false);
            }
        }
        i += 1;
    }
    if !listing {
        cmd.out(host, &super::help::text("arp"));
        return Outcome::Done(false);
    }
    let neighbors: Vec<crate::host::Neighbor> = host
        .neighbors()
        .into_iter()
        .filter(|n| wanted.is_none_or(|w| w == n.address))
        .collect();
    let adapters: Vec<_> = host
        .adapters()
        .into_iter()
        .filter(|a| a.ipv4.is_some() && interface.is_none_or(|i| a.ipv4 == Some(i)))
        .collect();
    if neighbors.is_empty() || adapters.is_empty() {
        cmd.out_line(host, "No ARP Entries Found.");
        return Outcome::Done(false);
    }
    for a in adapters {
        let ip = ip4(a.ipv4.unwrap());
        // The interface index in hex, as GetIpNetTable reports it.
        let index = if a.scope == 0 { 2 } else { a.scope };
        cmd.out_line(
            host,
            &format!(
                "\nInterface: {ip} --- 0x{index:x}\n  Internet Address      Physical Address      Type"
            ),
        );
        for n in &neighbors {
            let mac = n
                .mac
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<Vec<_>>()
                .join("-");
            let kind = if n.dynamic { "dynamic" } else { "static" };
            cmd.out_line(
                host,
                &format!("  {:<22}{:<22}{kind:<10}", ip4(n.address), mac),
            );
        }
    }
    done(true)
}
fn parse_ip4(text: &str) -> Option<[u8; 4]> {
    let mut out = [0u8; 4];
    let mut parts = text.split('.');
    for slot in &mut out {
        *slot = parts.next()?.parse().ok()?;
    }
    parts.next().is_none().then_some(out)
}
/// `netstat -s`: every family's statistics, or with `-p` one protocol's.
fn netstat_statistics(cmd: &mut Cmd, host: &mut dyn Host, protocol: Option<&str>) -> Option<()> {
    let (v4, v6) = host.statistics()?;
    let show = |name: &str| protocol.is_none_or(|p| p == name);
    let ip_rows = [
        "Packets Received",
        "Received Header Errors",
        "Received Address Errors",
        "Datagrams Forwarded",
        "Unknown Protocols Received",
        "Received Packets Discarded",
        "Received Packets Delivered",
        "Output Requests",
        "Routing Discards",
        "Discarded Output Packets",
        "Output Packet No Route",
        "Reassembly Required",
        "Reassembly Successful",
        "Reassembly Failures",
        "Datagrams Successfully Fragmented",
        "Datagrams Failing Fragmentation",
        "Fragments Created",
    ];
    for (name, title, s) in [("ip", "IPv4", &v4), ("ipv6", "IPv6", &v6)] {
        if show(name) {
            let mut text = format!("\n{title} Statistics\n");
            for (label, value) in ip_rows.iter().zip(s.ip) {
                text.push_str(&format!("\n  {label:<35}= {value}"));
            }
            cmd.out_line(host, &text);
        }
    }
    let icmp4: [(&str, u8); 13] = [
        ("Destination Unreachable", 3),
        ("Time Exceeded", 11),
        ("Parameter Problems", 12),
        ("Source Quenches", 4),
        ("Redirects", 5),
        ("Echo Replies", 0),
        ("Echos", 8),
        ("Timestamps", 13),
        ("Timestamp Replies", 14),
        ("Address Masks", 17),
        ("Address Mask Replies", 18),
        ("Router Solicitations", 10),
        ("Router Advertisements", 9),
    ];
    let icmp6: [(&str, u8); 15] = [
        ("Destination Unreachable", 1),
        ("Packet Too Big", 2),
        ("Time Exceeded", 3),
        ("Parameter Problems", 4),
        ("Echos", 128),
        ("Echo Replies", 129),
        ("MLD Queries", 130),
        ("MLD Reports", 131),
        ("MLD Dones", 132),
        ("Router Solicitations", 133),
        ("Router Advertisements", 134),
        ("Neighbor Solicitations", 135),
        ("Neighbor Advertisements", 136),
        ("Redirects", 137),
        ("Router Renumberings", 138),
    ];
    for (name, title, s, types) in [
        ("icmp", "ICMPv4", &v4, &icmp4[..]),
        ("icmpv6", "ICMPv6", &v6, &icmp6[..]),
    ] {
        if show(name) {
            let row = |label: &str, (r, t): (u32, u32)| format!("\n  {label:<26}{r:<12}{t:<10}");
            let mut text =
                format!("\n{title} Statistics\n\n                            Received    Sent");
            text.push_str(&row("Messages", s.icmp_messages));
            text.push_str(&row("Errors", s.icmp_errors));
            for (label, kind) in types {
                text.push_str(&row(label, s.icmp(*kind)));
            }
            cmd.out_line(host, &text);
        }
    }
    let tcp_rows = [
        "Active Opens",
        "Passive Opens",
        "Failed Connection Attempts",
        "Reset Connections",
        "Current Connections",
        "Segments Received",
        "Segments Sent",
        "Segments Retransmitted",
    ];
    for (name, title, s) in [("tcp", "IPv4", &v4), ("tcpv6", "IPv6", &v6)] {
        if show(name) {
            let mut text = format!("\nTCP Statistics for {title}\n");
            for (label, value) in tcp_rows.iter().zip(s.tcp) {
                text.push_str(&format!("\n  {label:<36}= {value}"));
            }
            cmd.out_line(host, &text);
        }
    }
    let udp_rows = [
        "Datagrams Received",
        "No Ports",
        "Receive Errors",
        "Datagrams Sent",
    ];
    for (name, title, s) in [("udp", "IPv4", &v4), ("udpv6", "IPv6", &v6)] {
        if show(name) {
            let mut text = format!("\nUDP Statistics for {title}\n");
            for (label, value) in udp_rows.iter().zip(s.udp) {
                text.push_str(&format!("\n  {label:<22}= {value}"));
            }
            cmd.out_line(host, &text);
        }
    }
    Some(())
}
fn netstat(cmd: &mut Cmd, host: &mut dyn Host, args: &[String]) -> Outcome {
    let mut all = false;
    let mut owner = false;
    let mut statistics = false;
    let mut protocol: Option<String> = None;
    let mut i = 0;
    while i < args.len() {
        let a = args[i].to_ascii_lowercase();
        let flags = a.trim_start_matches(['-', '/']);
        if flags == "p" {
            i += 1;
            protocol = args.get(i).map(|p| p.to_ascii_lowercase());
        } else if a.starts_with('-') || a.starts_with('/') {
            for f in flags.chars() {
                match f {
                    'a' => all = true,
                    'o' => owner = true,
                    's' => statistics = true,
                    'n' | 'f' | 'q' => (),
                    _ => {
                        cmd.out(host, &super::help::text("netstat"));
                        return Outcome::Done(false);
                    }
                }
            }
        } else {
            cmd.out(host, &super::help::text("netstat"));
            return Outcome::Done(false);
        }
        i += 1;
    }
    const PROTOCOLS: [&str; 8] = [
        "tcp", "udp", "tcpv6", "udpv6", "ip", "ipv6", "icmp", "icmpv6",
    ];
    let known = if statistics {
        &PROTOCOLS[..]
    } else {
        &PROTOCOLS[..4]
    };
    if protocol.as_deref().is_some_and(|p| !known.contains(&p)) {
        cmd.out(host, &super::help::text("netstat"));
        return Outcome::Done(false);
    }
    if statistics {
        if netstat_statistics(cmd, host, protocol.as_deref()).is_none() {
            cmd.out_line(host, "The network statistics are not available.");
            return Outcome::Done(false);
        }
        // `-s -p TCP` and `-s -p UDP` go on with that protocol's sockets;
        // the other protocols have no connection table.
        match protocol.as_deref() {
            Some("tcp" | "udp" | "tcpv6" | "udpv6") => all = true,
            _ => return done(true),
        }
    }
    let mut list = host.connections();
    // Windows' order: IPv4 TCP, IPv6 TCP, IPv4 UDP, IPv6 UDP.
    list.sort_by_key(|c| {
        (
            !c.tcp,
            c.v6.is_some(),
            c.v6.map(|e| e.local),
            c.local,
            c.local_port,
            c.v6.map(|e| e.remote),
            c.remote,
            c.remote_port,
        )
    });
    let pid_column = if owner { "     PID" } else { "" };
    cmd.out_line(
        host,
        &format!("\nActive Connections\n\n  Proto  Local Address          Foreign Address        State{pid_column}"),
    );
    // `[address%scope]:port` for IPv6, as Windows prints it.
    let end6 = |a: [u8; 16], scope: u32, port: u16| {
        let text = core::net::Ipv6Addr::from(a);
        if scope != 0 {
            format!("[{text}%{scope}]:{port}")
        } else {
            format!("[{text}]:{port}")
        }
    };
    for c in list {
        let name = match (c.tcp, c.v6.is_some()) {
            (true, false) => "tcp",
            (true, true) => "tcpv6",
            (false, false) => "udp",
            (false, true) => "udpv6",
        };
        if protocol.as_deref().is_some_and(|p| p != name) {
            continue;
        }
        if !all && (!c.tcp || c.state == 2) {
            continue;
        }
        let local = match c.v6 {
            Some(e) => end6(e.local, e.local_scope, c.local_port),
            None => format!("{}:{}", ip4(c.local), c.local_port),
        };
        let line = if c.tcp {
            let foreign = match c.v6 {
                Some(e) => end6(e.remote, e.remote_scope, c.remote_port),
                None => format!("{}:{}", ip4(c.remote), c.remote_port),
            };
            let state = tcp_state_name(c.state);
            if owner {
                format!("  TCP    {local:<23}{foreign:<23}{state:<16}{}", c.pid)
            } else {
                format!("  TCP    {local:<23}{foreign:<23}{state}")
            }
        } else if owner {
            format!("  UDP    {local:<23}{:<39}{}", "*:*", c.pid)
        } else {
            format!("  UDP    {local:<23}*:*")
        };
        cmd.out_line(host, line.trim_end());
    }
    done(true)
}
fn tcp_state_name(state: u8) -> &'static str {
    match state {
        1 => "CLOSED",
        2 => "LISTENING",
        3 => "SYN_SENT",
        4 => "SYN_RECEIVED",
        5 => "ESTABLISHED",
        6 => "FIN_WAIT_1",
        7 => "FIN_WAIT_2",
        8 => "CLOSE_WAIT",
        9 => "CLOSING",
        10 => "LAST_ACK",
        11 => "TIME_WAIT",
        _ => "DELETE_TCB",
    }
}
fn tasklist(cmd: &mut Cmd, host: &mut dyn Host, args: &[String]) -> Outcome {
    let lower: Vec<String> = args.iter().map(|a| a.to_ascii_lowercase()).collect();
    let csv = lower.windows(2).any(|w| w[0] == "/fo" && w[1] == "csv");
    let list = lower.windows(2).any(|w| w[0] == "/fo" && w[1] == "list");
    let no_header = lower.iter().any(|a| a == "/nh");
    let filter = lower.windows(2).find(|w| w[0] == "/fi").map(|w| unq(&w[1]));
    let mut procs = host.processes();
    if let Some(f) = &filter {
        let parts: Vec<&str> = f.split_whitespace().collect();
        if parts.len() >= 3 {
            let (key, op, value) = (parts[0], parts[1], parts[2..].join(" "));
            procs.retain(|p| {
                let hit = match key {
                    "imagename" => wild::matches(&value, &p.name),
                    "pid" => value.parse::<u32>().is_ok_and(|v| v == p.pid),
                    "sessionname" => p.session_name.eq_ignore_ascii_case(&value),
                    _ => true,
                };
                if op == "ne" { !hit } else { hit }
            });
        }
    }
    if procs.is_empty() {
        cmd.out_line(
            host,
            "INFO: No tasks are running which match the specified criteria.",
        );
        return done(true);
    }
    if csv {
        if !no_header {
            cmd.out_line(
                host,
                "\"Image Name\",\"PID\",\"Session Name\",\"Session#\",\"Mem Usage\"",
            );
        }
        for p in procs {
            cmd.out_line(
                host,
                &format!(
                    "\"{}\",\"{}\",\"{}\",\"{}\",\"{} K\"",
                    p.name,
                    p.pid,
                    p.session_name,
                    p.session,
                    grouped(p.memory_kb)
                ),
            );
        }
    } else if list {
        for p in procs {
            cmd.out_line(host, &format!("\nImage Name:   {}\nPID:          {}\nSession Name: {}\nSession#:     {}\nMem Usage:    {} K", p.name, p.pid, p.session_name, p.session, grouped(p.memory_kb)));
        }
    } else {
        if !no_header {
            cmd.out_line(host, "\nImage Name                     PID Session Name        Session#    Mem Usage\n========================= ======== ================ =========== ============");
        }
        for p in procs {
            let name: String = p.name.chars().take(25).collect();
            cmd.out_line(
                host,
                &format!(
                    "{name:<25}{:>9} {:<16}{:>12}{:>11} K",
                    p.pid,
                    p.session_name,
                    p.session,
                    grouped(p.memory_kb)
                ),
            );
        }
    }
    done(true)
}
fn taskkill(cmd: &mut Cmd, host: &mut dyn Host, args: &[String]) -> Outcome {
    let lower: Vec<String> = args.iter().map(|a| a.to_ascii_lowercase()).collect();
    let force = lower.iter().any(|a| a == "/f");
    let mut targets: Vec<(u32, String)> = Vec::new();
    let procs = host.processes();
    let mut ok = true;
    let mut i = 0;
    while i < lower.len() {
        match lower[i].as_str() {
            "/pid" => {
                i += 1;
                let pid = lower.get(i).and_then(|p| p.parse::<u32>().ok());
                match pid.and_then(|pid| procs.iter().find(|p| p.pid == pid)) {
                    Some(p) => targets.push((p.pid, p.name.clone())),
                    None => {
                        cmd.err_line(
                            host,
                            &format!(
                                "ERROR: The process \"{}\" not found.",
                                lower.get(i).cloned().unwrap_or_default()
                            ),
                        );
                        ok = false;
                    }
                }
            }
            "/im" => {
                i += 1;
                let pat = args.get(i).map(|s| unq(s)).unwrap_or_default();
                let hits: Vec<(u32, String)> = procs
                    .iter()
                    .filter(|p| wild::matches(&pat, &p.name))
                    .map(|p| (p.pid, p.name.clone()))
                    .collect();
                if hits.is_empty() {
                    cmd.err_line(host, &format!("ERROR: The process \"{pat}\" not found."));
                    ok = false;
                }
                targets.extend(hits);
            }
            _ => (),
        }
        i += 1;
    }
    if targets.is_empty() && ok {
        return err(
            cmd,
            host,
            "ERROR: Invalid syntax. Neither /FI nor /PID nor /IM were specified.\nType \"TASKKILL /?\" for usage.",
            1,
        );
    }
    for (pid, name) in targets {
        match host.terminate(pid, force) {
            Ok(()) if force => cmd.out_line(
                host,
                &format!("SUCCESS: The process \"{name}\" with PID {pid} has been terminated."),
            ),
            Ok(()) => cmd.out_line(
                host,
                &format!(
                    "SUCCESS: Sent termination signal to the process \"{name}\" with PID {pid}."
                ),
            ),
            Err(_) => {
                cmd.err_line(host, &format!("ERROR: The process \"{name}\" with PID {pid} could not be terminated.\nReason: Access is denied."));
                ok = false;
            }
        }
    }
    cmd.errorlevel = if ok { 0 } else { 128 };
    done(ok)
}
fn timeout(cmd: &mut Cmd, host: &mut dyn Host, args: &[String]) -> Outcome {
    let nobreak = args.iter().any(|a| a.eq_ignore_ascii_case("/nobreak"));
    let value = args
        .iter()
        .filter(|a| !a.eq_ignore_ascii_case("/t") && !a.eq_ignore_ascii_case("/nobreak"))
        .find_map(|a| a.parse::<i64>().ok());
    let Some(seconds) = value else {
        return err(
            cmd,
            host,
            "ERROR: Invalid syntax. Default option is not allowed more than '1' time(s).\nType \"TIMEOUT /?\" for usage.",
            1,
        );
    };
    if !(-1..=99999).contains(&seconds) {
        return err(
            cmd,
            host,
            "ERROR: Invalid value for timeout (/T) specified. Valid range is -1 to 99999.",
            1,
        );
    }
    if cmd.input().is_some() && !nobreak {
        return err(
            cmd,
            host,
            "ERROR: Input redirection is not supported, exiting the process immediately.",
            1,
        );
    }
    if seconds == -1 {
        cmd.out(host, "\nPress any key to continue ...");
        return Outcome::Wait(
            BuiltinPending::Tool(Pending::Timeout {
                remaining: u64::MAX,
                nobreak: false,
            }),
            Step::Input(InputKind::Key),
        );
    }
    let tail = if nobreak {
        "press CTRL+C to quit ..."
    } else {
        "press a key to continue ..."
    };
    cmd.out(host, &format!("\nWaiting for {seconds} seconds, {tail}"));
    if seconds == 0 {
        cmd.out(host, "\n");
        return done(true);
    }
    Outcome::Wait(
        BuiltinPending::Tool(Pending::Timeout {
            remaining: seconds as u64,
            nobreak,
        }),
        Step::Sleep {
            ms: 1000,
            keys: !nobreak,
        },
    )
}
fn choice(cmd: &mut Cmd, host: &mut dyn Host, args: &[String]) -> Outcome {
    let mut keys: Vec<char> = vec!['Y', 'N'];
    let mut hide = false;
    let mut sensitive = false;
    let mut timeout = None;
    let mut default = None;
    let mut message = String::new();
    let mut i = 0;
    while i < args.len() {
        let l = args[i].to_ascii_lowercase();
        let inline = String::from(args[i].get(2..).unwrap_or(""));
        let value = |i: &mut usize| -> String {
            if inline.trim_start_matches(':').is_empty() {
                *i += 1;
                args.get(*i).map(|s| unq(s)).unwrap_or_default()
            } else {
                unq(inline.trim_start_matches(':'))
            }
        };
        if l == "/cs" {
            sensitive = true;
        } else if l.starts_with("/c") {
            keys = value(&mut i).chars().collect();
        } else if l == "/n" {
            hide = true;
        } else if l.starts_with("/t") {
            timeout = value(&mut i).parse::<u64>().ok();
        } else if l.starts_with("/d") {
            default = value(&mut i).chars().next();
        } else if l.starts_with("/m") {
            message = value(&mut i);
            // The rest of the line is the message when unquoted.
            if !args[i].starts_with('"') {
                while let Some(more) = args.get(i + 1).filter(|a| !a.starts_with('/')) {
                    message.push(' ');
                    message.push_str(more);
                    i += 1;
                }
            }
        }
        i += 1;
    }
    if timeout.is_some() != default.is_some() {
        return err(
            cmd,
            host,
            "ERROR: Invalid syntax. /D can only be specified when /T is specified.",
            255,
        );
    }
    if !sensitive {
        keys = keys.iter().map(|c| c.to_ascii_uppercase()).collect();
    }
    let mut prompt = message.clone();
    if !hide {
        if !prompt.is_empty() {
            prompt.push(' ');
        }
        prompt.push('[');
        prompt.push_str(
            &keys
                .iter()
                .map(|c| String::from(*c))
                .collect::<Vec<_>>()
                .join(","),
        );
        prompt.push_str("]?");
    }
    cmd.out(host, &prompt);
    if let Some(input) = input_text(cmd) {
        let c = input.chars().next().unwrap_or('\n');
        return resume(
            cmd,
            host,
            Pending::Choice {
                keys,
                sensitive,
                default,
                remaining: 0,
            },
            Resume::Key(c),
        );
    }
    match timeout {
        Some(t) => Outcome::Wait(
            BuiltinPending::Tool(Pending::Choice {
                keys,
                sensitive,
                default,
                remaining: t.max(1),
            }),
            Step::Sleep {
                ms: 1000,
                keys: true,
            },
        ),
        None => Outcome::Wait(
            BuiltinPending::Tool(Pending::Choice {
                keys,
                sensitive,
                default,
                remaining: 0,
            }),
            Step::Input(InputKind::Key),
        ),
    }
}
fn systeminfo(cmd: &mut Cmd, host: &mut dyn Host) -> Outcome {
    let s = host.system();
    let name = host.computer_name();
    let row = |label: &str, value: &str| format!("{label:<27}{value}");
    let mut lines = vec![
        String::new(),
        row("Host Name:", &name),
        row("OS Name:", &s.os_name),
        row("OS Version:", &s.os_version),
        row("OS Manufacturer:", crate::COMPANY),
        row("OS Configuration:", "Standalone Workstation"),
        row("OS Build Type:", "Multiprocessor Free"),
        row("Registered Owner:", &s.owner),
        row("Registered Organization:", ""),
        row("Product ID:", &s.product_id),
        row(
            "Original Install Date:",
            &format!(
                "{}, {}",
                date(&s.install_date),
                time_seconds(&s.install_date)
            ),
        ),
        row(
            "System Boot Time:",
            &format!("{}, {}", date(&s.boot_time), time_seconds(&s.boot_time)),
        ),
        row("System Manufacturer:", &s.manufacturer),
        row("System Model:", &s.model),
        row("System Type:", &s.system_type),
        row(
            "Processor(s):",
            &format!("{} Processor(s) Installed.", s.processors.len()),
        ),
    ];
    for (i, p) in s.processors.iter().enumerate() {
        lines.push(format!("{:27}[{:02}]: {p}", "", i + 1));
    }
    lines.extend([
        row("BIOS Version:", &s.bios),
        row("Windows Directory:", "C:\\Windows"),
        row("System Directory:", "C:\\Windows\\system32"),
        row("Boot Device:", "\\Device\\HarddiskVolume1"),
        row("System Locale:", &s.locale),
        row("Input Locale:", &s.locale),
        row("Time Zone:", &s.time_zone),
        row(
            "Total Physical Memory:",
            &format!("{} MB", grouped(s.memory_total_mb)),
        ),
        row(
            "Available Physical Memory:",
            &format!("{} MB", grouped(s.memory_free_mb)),
        ),
        row("Domain:", "WORKGROUP"),
        row("Logon Server:", &format!("\\\\{name}")),
        row("Hotfix(s):", "N/A"),
        row(
            "Network Card(s):",
            &format!("{} NIC(s) Installed.", host.adapters().len()),
        ),
    ]);
    for l in lines {
        cmd.out_line(host, &l);
    }
    done(true)
}
fn reg_root(key: &str) -> String {
    let (root, rest) = key.split_once('\\').unwrap_or((key, ""));
    let root = match root.to_ascii_uppercase().as_str() {
        "HKLM" | "HKEY_LOCAL_MACHINE" => "HKLM",
        "HKCU" | "HKEY_CURRENT_USER" => "HKCU",
        "HKCR" | "HKEY_CLASSES_ROOT" => "HKCR",
        "HKU" | "HKEY_USERS" => "HKU",
        "HKCC" | "HKEY_CURRENT_CONFIG" => "HKCC",
        _ => return String::new(),
    };
    if rest.is_empty() {
        String::from(root)
    } else {
        format!("{root}\\{rest}")
    }
}
fn long_root(key: &str) -> String {
    let (root, rest) = key.split_once('\\').unwrap_or((key, ""));
    let long = match root {
        "HKLM" => "HKEY_LOCAL_MACHINE",
        "HKCU" => "HKEY_CURRENT_USER",
        "HKCR" => "HKEY_CLASSES_ROOT",
        "HKU" => "HKEY_USERS",
        _ => "HKEY_CURRENT_CONFIG",
    };
    if rest.is_empty() {
        String::from(long)
    } else {
        format!("{long}\\{rest}")
    }
}
fn reg_text(v: &RegValue) -> String {
    match v {
        RegValue::Sz(s) | RegValue::ExpandSz(s) => s.clone(),
        RegValue::MultiSz(v) => v.join("\\0"),
        RegValue::Dword(d) => format!("0x{d:x}"),
        RegValue::Qword(q) => format!("0x{q:x}"),
        RegValue::Binary(b) => b.iter().map(|x| format!("{x:02X}")).collect(),
        RegValue::None => String::new(),
    }
}
fn reg(cmd: &mut Cmd, host: &mut dyn Host, args: &[String]) -> Outcome {
    let Some(op) = args.first().map(|a| a.to_ascii_lowercase()) else {
        return err(
            cmd,
            host,
            "ERROR: Invalid syntax.\nType \"REG /?\" for usage.",
            1,
        );
    };
    let Some(key) = args
        .get(1)
        .map(|k| reg_root(&unq(k)))
        .filter(|k| !k.is_empty())
    else {
        return err(
            cmd,
            host,
            "ERROR: Invalid key name.\nType \"REG /?\" for usage.",
            1,
        );
    };
    let lower: Vec<String> = args.iter().map(|a| a.to_ascii_lowercase()).collect();
    let flag = |f: &str| lower.iter().position(|a| a == f);
    let value_name = flag("/v")
        .and_then(|i| args.get(i + 1))
        .map(|v| unq(v))
        .or_else(|| flag("/ve").map(|_| String::new()));
    match op.as_str() {
        "query" => {
            let values = match host.reg_values(&key) {
                Ok(v) => v,
                Err(_) => {
                    return err(
                        cmd,
                        host,
                        "ERROR: The system was unable to find the specified registry key or value.",
                        1,
                    );
                }
            };
            cmd.out_line(host, &format!("\n{}", long_root(&key)));
            let mut any = false;
            for (n, v) in values {
                if value_name
                    .as_ref()
                    .is_some_and(|w| !w.eq_ignore_ascii_case(&n))
                {
                    continue;
                }
                any = true;
                let shown = if n.is_empty() {
                    String::from("(Default)")
                } else {
                    n
                };
                cmd.out_line(
                    host,
                    &format!("    {shown}    {}    {}", v.type_name(), reg_text(&v)),
                );
            }
            if value_name.is_none() {
                for k in host.reg_subkeys(&key).unwrap_or_default() {
                    cmd.out_line(host, &format!("\n{}\\{k}", long_root(&key)));
                }
            } else if !any {
                return err(
                    cmd,
                    host,
                    "ERROR: The system was unable to find the specified registry key or value.",
                    1,
                );
            }
            cmd.out(host, "\n");
            done(true)
        }
        "add" => {
            let kind = flag("/t")
                .and_then(|i| args.get(i + 1))
                .map(|t| t.to_ascii_uppercase())
                .unwrap_or_else(|| String::from("REG_SZ"));
            let data = flag("/d")
                .and_then(|i| args.get(i + 1))
                .map(|d| unq(d))
                .unwrap_or_default();
            let value = match kind.as_str() {
                "REG_SZ" => RegValue::Sz(data),
                "REG_EXPAND_SZ" => RegValue::ExpandSz(data),
                "REG_MULTI_SZ" => RegValue::MultiSz(data.split("\\0").map(String::from).collect()),
                "REG_DWORD" => match parse_num(&data) {
                    Some(v) => RegValue::Dword(v as u32),
                    None => {
                        return err(
                            cmd,
                            host,
                            "ERROR: Invalid syntax.\nType \"REG ADD /?\" for usage.",
                            1,
                        );
                    }
                },
                "REG_QWORD" => match parse_num(&data) {
                    Some(v) => RegValue::Qword(v),
                    None => {
                        return err(
                            cmd,
                            host,
                            "ERROR: Invalid syntax.\nType \"REG ADD /?\" for usage.",
                            1,
                        );
                    }
                },
                "REG_BINARY" => RegValue::Binary(
                    (0..data.len() / 2)
                        .filter_map(|i| u8::from_str_radix(&data[i * 2..i * 2 + 2], 16).ok())
                        .collect(),
                ),
                "REG_NONE" => RegValue::None,
                _ => {
                    return err(
                        cmd,
                        host,
                        "ERROR: Invalid syntax.\nType \"REG ADD /?\" for usage.",
                        1,
                    );
                }
            };
            match host.reg_set(&key, value_name.as_deref().unwrap_or(""), &value) {
                Ok(()) => {
                    cmd.out_line(host, "The operation completed successfully.");
                    done(true)
                }
                Err(FsError::Denied) => err(cmd, host, "ERROR: Access is denied.", 1),
                Err(e) => err(cmd, host, &format!("ERROR: {}", e.message()), 1),
            }
        }
        "delete" => {
            let all = flag("/va").is_some();
            let target = if all { None } else { value_name.as_deref() };
            match host.reg_delete(&key, target) {
                Ok(()) => {
                    cmd.out_line(host, "The operation completed successfully.");
                    done(true)
                }
                Err(FsError::Denied) => err(cmd, host, "ERROR: Access is denied.", 1),
                Err(_) => err(
                    cmd,
                    host,
                    "ERROR: The system was unable to find the specified registry key or value.",
                    1,
                ),
            }
        }
        "export" | "save" | "load" | "unload" | "import" | "restore" | "copy" | "compare"
        | "flags" => err(cmd, host, "ERROR: Access is denied.", 1),
        _ => err(
            cmd,
            host,
            &format!(
                "ERROR: Invalid argument/option - '{}'.\nType \"REG /?\" for usage.",
                args[0]
            ),
            1,
        ),
    }
}
fn parse_num(s: &str) -> Option<u64> {
    if let Some(h) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        u64::from_str_radix(h, 16).ok()
    } else {
        s.parse().ok()
    }
}
fn setx(cmd: &mut Cmd, host: &mut dyn Host, args: &[String]) -> Outcome {
    let machine = args.iter().any(|a| a.eq_ignore_ascii_case("/m"));
    let plain: Vec<String> = args
        .iter()
        .filter(|a| !a.starts_with('/'))
        .map(|a| unq(a))
        .collect();
    if plain.len() != 2 {
        return err(
            cmd,
            host,
            "ERROR: Invalid syntax. Default option is not allowed more than '2' time(s).\nType \"SETX /?\" for usage.",
            1,
        );
    }
    if machine {
        return err(
            cmd,
            host,
            "ERROR: Access to the registry path is denied.",
            1,
        );
    }
    let value = if plain[1].len() > 1024 {
        String::from(&plain[1][..1024])
    } else {
        plain[1].clone()
    };
    match host.set_user_variable(&plain[0], &value) {
        Ok(()) => {
            if plain[1].len() > 1024 {
                cmd.out_line(
                    host,
                    "\nWARNING: The data being saved is truncated to 1024 characters.",
                );
            }
            cmd.out_line(host, "\nSUCCESS: Specified value was saved.");
            done(true)
        }
        Err(_) => err(
            cmd,
            host,
            "ERROR: Access to the registry path is denied.",
            1,
        ),
    }
}
fn certutil(cmd: &mut Cmd, host: &mut dyn Host, args: &[String]) -> Outcome {
    let lower: Vec<String> = args.iter().map(|a| a.to_ascii_lowercase()).collect();
    if lower
        .first()
        .is_some_and(|a| a == "-hashfile" || a == "/hashfile")
    {
        let Some(file) = args.get(1).map(|f| unq(f)) else {
            return err(
                cmd,
                host,
                "CertUtil: -hashfile command FAILED: 0x80070057 (WIN32: 87 ERROR_INVALID_PARAMETER)\nCertUtil: The parameter is incorrect.",
                87,
            );
        };
        let algorithm = args
            .get(2)
            .map(|a| a.to_ascii_uppercase())
            .unwrap_or_else(|| String::from("SHA1"));
        let Ok(native) = cmd.native(host, &file) else {
            return err(
                cmd,
                host,
                "CertUtil: -hashfile command FAILED: 0x80070002 (WIN32: 2 ERROR_FILE_NOT_FOUND)\nCertUtil: The system cannot find the file specified.",
                2,
            );
        };
        match host.digest(&native, &algorithm) {
            Ok(d) => {
                let hex: String = d.iter().map(|b| format!("{b:02x}")).collect();
                cmd.out_line(host, &format!("{algorithm} hash of {file}:\n{hex}\nCertUtil: -hashfile command completed successfully."));
                done(true)
            }
            Err(FsError::NotFound) => err(
                cmd,
                host,
                "CertUtil: -hashfile command FAILED: 0x80070002 (WIN32: 2 ERROR_FILE_NOT_FOUND)\nCertUtil: The system cannot find the file specified.",
                2,
            ),
            Err(_) => err(
                cmd,
                host,
                &format!(
                    "CertUtil: -hashfile command FAILED: 0x80090008 (-2146893816 NTE_BAD_ALGID)\nCertUtil: Invalid algorithm specified ({algorithm})."
                ),
                1,
            ),
        }
    } else {
        cmd.out_line(host, "CertUtil: -dump command completed successfully.");
        done(true)
    }
}
fn forfiles(cmd: &mut Cmd, host: &mut dyn Host, args: &[String]) -> Outcome {
    let lower: Vec<String> = args.iter().map(|a| a.to_ascii_lowercase()).collect();
    let get = |f: &str| {
        lower
            .iter()
            .position(|a| a == f)
            .and_then(|i| args.get(i + 1))
            .map(|v| unq(v))
    };
    let path = get("/p").unwrap_or_else(|| cmd.cwd.clone());
    let mask = get("/m").unwrap_or_else(|| String::from("*"));
    let recurse = lower.iter().any(|a| a == "/s");
    let command = get("/c").unwrap_or_else(|| String::from("cmd /c echo @file"));
    let Ok(root) = cmd.full(&path) else {
        return err(cmd, host, "ERROR: The directory name is invalid.", 1);
    };
    let mut files = Vec::new();
    collect_all(host, &root, &mask, recurse, false, &mut files);
    if files.is_empty() {
        return err(
            cmd,
            host,
            "ERROR: No files found with the specified search criteria.",
            1,
        );
    }
    // Each command runs as `cmd /c` with @file, @fname, @ext, @path.
    for (p, m) in files {
        let name = wpath::name(&p);
        let line = command
            .replace("@file", &format!("\"{name}\""))
            .replace("@fname", &format!("\"{}\"", wpath::stem(name)))
            .replace(
                "@ext",
                &format!("\"{}\"", wpath::extension(name).trim_start_matches('.')),
            )
            .replace("@path", &format!("\"{p}\""))
            .replace("@isdir", if m.dir { "TRUE" } else { "FALSE" })
            .replace("@fsize", &format!("{}", m.size));
        let inner = line.strip_prefix("cmd /c ").unwrap_or(&line);
        if let Some(text) = inner.strip_prefix("echo ") {
            cmd.out_line(host, text.trim());
        }
    }
    done(true)
}
fn net(cmd: &mut Cmd, host: &mut dyn Host, args: &[String]) -> Outcome {
    let sub = args
        .first()
        .map(|a| a.to_ascii_lowercase())
        .unwrap_or_default();
    match sub.as_str() {
        "user" | "users" if args.len() == 1 => {
            let name = host.computer_name();
            let user = host.user_name();
            cmd.out_line(host, &format!("\nUser accounts for \\\\{name}\n\n-------------------------------------------------------------------------------\nAdministrator            DefaultAccount           Guest                    \n{user:<25}WDAGUtilityAccount       \nThe command completed successfully.\n"));
            done(true)
        }
        "use" => {
            cmd.out_line(
                host,
                "New connections will be remembered.\n\nThere are no entries in the list.\n",
            );
            done(true)
        }
        "start" | "stop" => {
            let name = args.get(1).map(|n| unq(n));
            super::sc::net_service(cmd, host, sub == "stop", name.as_deref())
        }
        "config" | "statistics" => {
            let name = host.computer_name();
            cmd.out_line(host, &format!("\nWorkstation Statistics for \\\\{name}\n\nThe command completed successfully.\n"));
            done(true)
        }
        "" => {
            cmd.out_line(host, "The syntax of this command is:\n\nNET\n    [ ACCOUNTS | COMPUTER | CONFIG | CONTINUE | FILE | GROUP | HELP |\n      HELPMSG | LOCALGROUP | PAUSE | SESSION | SHARE | START |\n      STATISTICS | STOP | TIME | USE | USER | VIEW ]\n");
            done(true)
        }
        "session" if host.elevated() => {
            cmd.out_line(host, "There are no entries in the list.\n");
            done(true)
        }
        _ => err(
            cmd,
            host,
            "System error 5 has occurred.\n\nAccess is denied.\n",
            2,
        ),
    }
}
/// The administrator tools in an elevated Command Prompt. Doors checks and
/// repairs its volume and system image itself (the storage service at every
/// mount, the kernel's authenticated initfs at boot), so these report what
/// is true rather than simulate a scan.
fn elevated_tool(cmd: &mut Cmd, host: &mut dyn Host, tool: &str, args: &[String]) -> Outcome {
    let first = args
        .first()
        .map(|a| a.to_ascii_lowercase())
        .unwrap_or_default();
    match tool {
        "sfc" if first == "/scannow" || first == "/verifyonly" => {
            cmd.out_line(host, "\nBeginning system scan.  This process will take some time.\n\nDoors Resource Protection verified the system image: Doors starts only from\nits authenticated system image, so there are no integrity violations to repair.");
            done(true)
        }
        "sfc" => {
            cmd.out_line(host, "\nMacrohard (R) Doors (R) Resource Checker Version 6.0\nCopyright (C) Macrohard Corporation. All rights reserved.\n\nScans the integrity of all protected system files.\n\nSFC [/SCANNOW] [/VERIFYONLY]\n\n/SCANNOW        Scans integrity of all protected system files.\n/VERIFYONLY     Scans integrity of all protected system files. No repair\n                operation is performed.");
            done(true)
        }
        "dism" => {
            cmd.out_line(host, "\nDeployment Image Servicing and Management tool\nVersion: 10.0.26300.1\n\nImage Version: 10.0.26300.1\n\nThe Doors system image is read-only and authenticated at boot; optional\nfeatures are managed in Settings > System > Optional features.\nThe operation completed successfully.");
            done(true)
        }
        "chkdsk" => {
            cmd.out_line(host, "The type of the file system is NTFS.\n\nDoors' storage service validates this volume every time it is mounted and\nreplays its journal after an interrupted shutdown; no errors were found.");
            done(true)
        }
        _ => {
            // fsutil
            if first == "fsinfo"
                && args
                    .get(1)
                    .is_some_and(|a| a.eq_ignore_ascii_case("drives"))
            {
                cmd.out_line(host, "\nDrives: C:\\");
            } else {
                cmd.out_line(host, "---- Commands Supported ----\n\n8dot3name         8dot3name management\nbehavior          Control file system behavior\nfile              File specific commands\nfsInfo            File system information\nhardlink          Hardlink management\nvolume            Volume management");
            }
            done(true)
        }
    }
}
fn wmic(cmd: &mut Cmd, host: &mut dyn Host, args: &[String]) -> Outcome {
    let joined = args.join(" ").to_ascii_lowercase();
    if joined.starts_with("os get caption") {
        let s = host.system();
        cmd.out_line(host, &format!("Caption\n{}\n", s.os_name));
        return done(true);
    }
    if joined.starts_with("cpu get name") {
        let s = host.system();
        cmd.out_line(host, "Name");
        for p in s.processors {
            cmd.out_line(host, &p);
        }
        return done(true);
    }
    if joined.starts_with("computersystem get name") {
        let n = host.computer_name();
        cmd.out_line(host, &format!("Name\n{n}\n"));
        return done(true);
    }
    err(
        cmd,
        host,
        "Node - localhost\nERROR:\nDescription = Invalid class\n",
        1,
    )
}
/// `cmd /c string` and `cmd /k string`: run the command line in a child
/// interpreter context (its SETs end with it, as with a child cmd.exe).
fn nested(cmd: &mut Cmd, host: &mut dyn Host, tail: &str) -> Outcome {
    let t = tail.trim_start();
    let mut rest = t;
    let mut run = None;
    let mut delayed = cmd.delayed;
    while let Some(s) = rest.strip_prefix('/') {
        let (word, after) = s.split_once(' ').unwrap_or((s, ""));
        match word.to_ascii_lowercase().as_str() {
            "c" | "k" => {
                run = Some(after);
                break;
            }
            "v:on" => delayed = true,
            "v:off" => delayed = false,
            "q" | "d" | "s" | "a" | "u" | "e:on" | "e:off" | "f:on" | "f:off" => (),
            _ => {
                if let Some(cmdline) = word
                    .to_ascii_lowercase()
                    .strip_prefix("c")
                    .filter(|w| w.starts_with('"'))
                {
                    let _ = cmdline;
                }
            }
        }
        rest = after.trim_start();
    }
    let Some(line) = run else {
        let banner = super::Cmd::banner();
        cmd.out_line(host, &banner);
        return done(true);
    };
    // `cmd /c "line"`: outer quotes go when the line is quoted whole.
    let line = line.trim();
    let line = if line.len() >= 2
        && line.starts_with('"')
        && line.ends_with('"')
        && line[1..line.len() - 1].matches('"').count() % 2 == 0
    {
        &line[1..line.len() - 1]
    } else {
        line
    };
    cmd.push_child(host, line, delayed)
}
