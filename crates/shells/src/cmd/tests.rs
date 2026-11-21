//! cmd.exe behaviour on the in-memory host, with expected text from the
//! Windows 11 reference capture where formats matter.
use super::*;
use crate::memhost::MemHost;
use crate::{Resume, Step};
use std::string::ToString;

const HOME: &str = "C:\\Users\\1001";

fn engine() -> Cmd {
    let env = default_environment("Win", "DOORS-PC", HOME, 4);
    Cmd::new(env, HOME)
}
/// Drive the engine to idle, answering prompts with `answers` (then 'y'),
/// returning the console text.
fn drive(cmd: &mut Cmd, host: &mut MemHost, mut step: Step, answers: &mut Vec<Resume>) -> String {
    let mut out = String::new();
    for _ in 0..10_000 {
        for e in cmd.take_events() {
            if let Event::Text(t) = e {
                out.push_str(&t);
            }
        }
        step = match step {
            Step::Idle | Step::Exit(_) => break,
            Step::Input(_) => {
                let r = if answers.is_empty() {
                    Resume::Key('y')
                } else {
                    answers.remove(0)
                };
                cmd.resume(host, r)
            }
            Step::Sleep { .. } => cmd.resume(host, Resume::Tick),
            Step::Spawn(_) => cmd.resume(
                host,
                Resume::Exited {
                    code: 0,
                    output: Vec::new(),
                },
            ),
            Step::Launch(_) => cmd.resume(host, Resume::Done(true)),
        };
    }
    for e in cmd.take_events() {
        if let Event::Text(t) = e {
            out.push_str(&t);
        }
    }
    out
}
fn batch(host: &mut MemHost, script: &str) -> String {
    host.file("/users/1001/t.bat", script);
    let mut cmd = engine();
    let step = cmd.start_batch(host, "C:\\Users\\1001\\t.bat", &[]);
    drive(&mut cmd, host, step, &mut Vec::new())
}
fn prompt(cmd: &mut Cmd, host: &mut MemHost, line: &str) -> String {
    let step = cmd.submit(host, line);
    drive(cmd, host, step, &mut Vec::new())
}
fn work(host: &mut MemHost) {
    host.dir("/users/1001/work");
    host.file("/users/1001/work/a.txt", "hello world\r\nsecond line\r\n");
    host.file("/users/1001/work/b.log", "Hello again\r\n");
    host.dir("/users/1001/work/sub");
    host.file("/users/1001/work/sub/c.txt", "nested\r\n");
}

#[test]
fn dir_matches_the_windows_layout() {
    let mut h = MemHost::new();
    work(&mut h);
    let mut c = engine();
    prompt(&mut c, &mut h, "cd work");
    let out = prompt(&mut c, &mut h, "dir");
    let expected = " Volume in drive C has no label.\n Volume Serial Number is B886-8105\n\n Directory of C:\\Users\\1001\\work\n\n11/02/2026  12:16 PM    <DIR>          .\n11/02/2026  12:16 PM    <DIR>          ..\n11/02/2026  12:16 PM                26 a.txt\n11/02/2026  12:16 PM                13 b.log\n11/02/2026  12:16 PM    <DIR>          sub\n               2 File(s)             39 bytes\n               3 Dir(s)  33,532,461,056 bytes free\n";
    assert_eq!(out, expected);
    assert_eq!(prompt(&mut c, &mut h, "dir /b"), "a.txt\nb.log\nsub\n");
    let missing = prompt(&mut c, &mut h, "dir nothere.xyz");
    assert!(
        missing.ends_with(" Volume Serial Number is B886-8105\nFile Not Found\n"),
        "{missing}"
    );
    let s = prompt(&mut c, &mut h, "dir /s");
    assert!(
        s.contains(" Directory of C:\\Users\\1001\\work\\sub\n\n"),
        "{s}"
    );
    assert!(s.contains("     Total Files Listed:\n               3 File(s)             47 bytes\n               5 Dir(s)  33,532,461,056 bytes free\n"), "{s}");
    let wide = prompt(&mut c, &mut h, "dir /w");
    assert!(
        wide.contains("[.]     [..]    a.txt   b.log   [sub]   \n"),
        "{wide}"
    );
}

#[test]
fn echo_set_and_expansion() {
    let mut h = MemHost::new();
    let out = batch(
        &mut h,
        "@echo off\necho hello  world\necho.\necho\nset NAME=Doors OS\necho %NAME:~0,5% [%name:OS=Windows%]\nset /a x=7, y=x^<^<2, x %%= 4\necho %x% %y%\nset NOTHERE_ZZZ\necho 100%%\n",
    );
    assert_eq!(
        out,
        "hello  world\n\nECHO is off.\nDoors [Doors Windows]\n3 28\nEnvironment variable NOTHERE_ZZZ not defined\n100%\n"
    );
    let mut c = engine();
    assert_eq!(prompt(&mut c, &mut h, "set /a 3*4+1"), "13");
    assert_eq!(
        prompt(&mut c, &mut h, "echo %undefined_var%"),
        "%undefined_var%\n"
    );
}

#[test]
fn echo_on_shows_each_command_with_the_prompt() {
    let mut h = MemHost::new();
    let out = batch(&mut h, "echo hello\n@echo off\necho done\n");
    assert_eq!(out, "\nC:\\Users\\1001>echo hello \nhello\ndone\n");
}

#[test]
fn if_for_and_errors_match_the_reference() {
    let mut h = MemHost::new();
    work(&mut h);
    h.file(
        "/users/1001/work/f.txt",
        "hello world\r\nsecond line\r\nHello again\r\n",
    );
    let out = batch(
        &mut h,
        "@echo off\ncd work\nif exist a.txt (echo yes) else (echo no)\nif /i \"A\"==\"a\" echo same\nif 10 gtr 9 echo greater\nif not exist zz.txt echo missing & echo both\nfor %%i in (*.txt) do @echo file %%i\nfor /l %%i in (1,2,5) do @echo n %%i\nfor /f \"tokens=1,2\" %%a in (\"one two three\") do @echo %%b-%%a\nfor /f \"delims=\" %%l in ('type a.txt') do @echo L=%%l\nfor %%i in (a.txt) do @echo %%~ni %%~xi %%~zi\nmkdir sub\nrd nothere\ndel nothere.txt\ncd nothere\nnothere_command\necho %errorlevel%\n",
    );
    let expected = "yes\nsame\ngreater\nmissing \nboth\nfile a.txt\nfile f.txt\nn 1\nn 3\nn 5\ntwo-one\nL=hello world\nL=second line\na .txt 26\nA subdirectory or file sub already exists.\nThe system cannot find the file specified.\nCould Not Find C:\\Users\\1001\\work\\nothere.txt\nThe system cannot find the path specified.\n'nothere_command' is not recognized as an internal or external command,\noperable program or batch file.\n9009\n";
    assert_eq!(out, expected);
}

#[test]
fn goto_call_shift_and_setlocal() {
    let mut h = MemHost::new();
    let out = batch(
        &mut h,
        "@echo off\nset A=outer\ncall :sub one \"two three\"\necho back %A% %R%\ngoto :skip\necho never\n:skip\nsetlocal\nset A=inner\nendlocal\necho after %A%\ngoto :eof\n:sub\necho in %1 %~2\nshift\necho shifted %1\nset R=done\nexit /b 3\n",
    );
    assert_eq!(
        out,
        "in one two three\nshifted \"two three\"\nback outer done\nafter outer\n"
    );
}

#[test]
fn delayed_expansion_in_loops() {
    let mut h = MemHost::new();
    let out = batch(
        &mut h,
        "@echo off\nsetlocal enabledelayedexpansion\nset n=0\nfor %%i in (a b c) do (\n  set /a n+=1\n  echo !n! %%i\n)\necho total %n%\n",
    );
    assert_eq!(out, "1 a\n2 b\n3 c\ntotal 3\n");
}

#[test]
fn redirection_pipes_and_conditionals() {
    let mut h = MemHost::new();
    work(&mut h);
    let mut c = engine();
    prompt(&mut c, &mut h, "cd work");
    assert_eq!(
        prompt(&mut c, &mut h, "echo one> out.txt & echo two >>out.txt"),
        ""
    );
    // cmd.exe keeps the space before `&`: the file gets "one ".
    assert_eq!(
        h.text("/users/1001/work/out.txt").unwrap(),
        "one \r\ntwo \r\n"
    );
    assert_eq!(
        prompt(&mut c, &mut h, "type a.txt | find \"second\""),
        "second line\n"
    );
    assert_eq!(
        prompt(&mut c, &mut h, "type a.txt | sort /r"),
        "second line\nhello world\n"
    );
    assert_eq!(
        prompt(&mut c, &mut h, "dir nothere 2>nul || echo failed"),
        " Volume in drive C has no label.\n Volume Serial Number is B886-8105\nfailed\n"
    );
    assert_eq!(prompt(&mut c, &mut h, "type a.txt >nul && echo ok"), "ok\n");
    assert_eq!(prompt(&mut c, &mut h, "(echo x& echo y) > block.txt"), "");
    assert_eq!(h.text("/users/1001/work/block.txt").unwrap(), "x\r\ny\r\n");
    assert_eq!(
        prompt(&mut c, &mut h, "sort < a.txt"),
        "hello world\nsecond line\n"
    );
}

#[test]
fn file_commands_report_like_windows() {
    let mut h = MemHost::new();
    work(&mut h);
    let mut c = engine();
    prompt(&mut c, &mut h, "cd work");
    assert_eq!(
        prompt(&mut c, &mut h, "copy a.txt d.txt"),
        "        1 file(s) copied.\n"
    );
    assert_eq!(
        prompt(&mut c, &mut h, "copy a.txt+b.log e.txt"),
        "a.txt\nb.log\n        1 file(s) copied.\n"
    );
    assert_eq!(
        h.text("/users/1001/work/e.txt").unwrap(),
        "hello world\r\nsecond line\r\nHello again\r\n"
    );
    assert_eq!(
        prompt(&mut c, &mut h, "copy nothere.txt x.txt"),
        "The system cannot find the file specified.\n        0 file(s) copied.\n"
    );
    assert_eq!(
        prompt(&mut c, &mut h, "move d.txt sub"),
        "        1 file(s) moved.\n"
    );
    assert!(h.text("/users/1001/work/sub/d.txt").is_some());
    assert_eq!(prompt(&mut c, &mut h, "ren e.txt f.txt"), "");
    assert_eq!(
        prompt(&mut c, &mut h, "ren f.txt b.log"),
        "A duplicate file name exists, or the file\ncannot be found.\n"
    );
    assert_eq!(
        prompt(&mut c, &mut h, "type a.txt"),
        "hello world\nsecond line\n"
    );
    assert_eq!(prompt(&mut c, &mut h, "md new\\deep\\dir"), "");
    assert!(h.files.contains_key("/users/1001/work/new/deep/dir"));
    assert_eq!(
        prompt(&mut c, &mut h, "rd new"),
        "The directory is not empty.\n"
    );
    assert_eq!(prompt(&mut c, &mut h, "rd /s /q new"), "");
    assert!(!h.files.contains_key("/users/1001/work/new"));
    // del * asks first; 'y' answers.
    assert_eq!(
        prompt(&mut c, &mut h, "del sub"),
        "C:\\Users\\1001\\work\\sub\\*, Are you sure (Y/N)? "
    );
    assert!(h.text("/users/1001/work/sub/c.txt").is_none());
    assert_eq!(
        prompt(&mut c, &mut h, "pushd sub & cd & popd & cd"),
        "C:\\Users\\1001\\work\\sub\nC:\\Users\\1001\\work\n"
    );
}

#[test]
fn text_tools_match_the_reference() {
    let mut h = MemHost::new();
    work(&mut h);
    h.file(
        "/users/1001/work/f.txt",
        "hello world\r\nsecond line\r\nHello again\r\n",
    );
    let mut c = engine();
    prompt(&mut c, &mut h, "cd work");
    assert_eq!(
        prompt(&mut c, &mut h, "find \"hello\" a.txt b.log"),
        "\n---------- A.TXT\nhello world\n\n---------- B.LOG\n"
    );
    assert_eq!(
        prompt(&mut c, &mut h, "find /c \"l\" a.txt"),
        "\n---------- A.TXT: 2\n"
    );
    assert_eq!(
        prompt(&mut c, &mut h, "find /i /n \"HELLO\" a.txt"),
        "\n---------- A.TXT\n[1]hello world\n"
    );
    assert_eq!(
        prompt(&mut c, &mut h, "findstr /n /i \"hello\" *.txt *.log"),
        "a.txt:1:hello world\nf.txt:1:hello world\nf.txt:3:Hello again\nb.log:1:Hello again\n"
    );
    assert_eq!(
        prompt(&mut c, &mut h, "findstr /s \"nested\" *"),
        "sub\\c.txt:nested\n"
    );
    assert_eq!(
        prompt(&mut c, &mut h, "findstr /r \"^s.*e$\" a.txt"),
        "second line\n"
    );
    assert_eq!(
        prompt(&mut c, &mut h, "fc a.txt b.log"),
        "Comparing files a.txt and B.LOG\n***** a.txt\nhello world\nsecond line\n***** B.LOG\nHello again\n*****\n\n"
    );
    assert_eq!(
        prompt(&mut c, &mut h, "fc a.txt a.txt"),
        "Comparing files a.txt and A.TXT\nFC: no differences encountered\n\n"
    );
    assert_eq!(
        prompt(&mut c, &mut h, "tree /a"),
        "Folder PATH listing\nVolume serial number is B886-8105\nC:.\n\\---sub\n"
    );
    assert_eq!(
        prompt(&mut c, &mut h, "where cmd"),
        "C:\\Windows\\System32\\cmd.exe\n"
    );
    assert_eq!(
        prompt(&mut c, &mut h, "where nothere"),
        "INFO: Could not find files for the given pattern(s).\n"
    );
    assert_eq!(prompt(&mut c, &mut h, "hostname"), "DOORS-PC\n");
    assert_eq!(prompt(&mut c, &mut h, "whoami"), "doors-pc\\win\n");
    assert_eq!(prompt(&mut c, &mut h, "assoc .txt"), ".txt=txtfilelegacy\n");
    assert_eq!(prompt(&mut c, &mut h, "date /t"), "Mon 11/02/2026 \n");
    assert_eq!(prompt(&mut c, &mut h, "time /t"), "12:16 PM\n");
    assert_eq!(prompt(&mut c, &mut h, "attrib +r f.txt"), "");
    let listing = prompt(&mut c, &mut h, "attrib");
    assert!(
        listing.contains("A    R               C:\\Users\\1001\\work\\f.txt\n"),
        "{listing}"
    );
    let x = prompt(&mut c, &mut h, "xcopy sub sub2 /e /i");
    assert_eq!(x, "sub\\c.txt\n1 File(s) copied\n");
}

#[test]
fn legacy_commands_open_doors_apps_and_settings() {
    let mut h = MemHost::new();
    let mut c = engine();
    assert!(matches!(
        c.submit(&mut h, "notepad readme.txt"),
        Step::Launch(crate::Launch::App {
            app: crate::legacy::App::Notes,
            ..
        })
    ));
    let mut c = engine();
    assert!(matches!(
        c.submit(&mut h, "control printers"),
        Step::Launch(crate::Launch::Settings(_))
    ));
    let mut c = engine();
    assert!(matches!(
        c.submit(&mut h, "start ms-settings:display"),
        Step::Launch(crate::Launch::Settings(_))
    ));
    let mut c = engine();
    assert!(matches!(
        c.submit(&mut h, "start https://example.com"),
        Step::Launch(crate::Launch::Url(_))
    ));
    let mut c = engine();
    assert!(matches!(
        c.submit(&mut h, "explorer ."),
        Step::Launch(crate::Launch::App {
            app: crate::legacy::App::Files,
            ..
        })
    ));
}

#[test]
fn child_cmd_and_exit_codes() {
    let mut h = MemHost::new();
    let mut c = engine();
    assert_eq!(
        prompt(&mut c, &mut h, "cmd /c \"set CHILD=1 & exit 5\""),
        ""
    );
    assert_eq!(
        prompt(&mut c, &mut h, "echo %errorlevel% [%CHILD%]"),
        "5 [%CHILD%]\n"
    );
    assert_eq!(
        prompt(&mut c, &mut h, "ver"),
        "\nMacrohard Doors [Version 10.0.26300.1]\n"
    );
    assert!(matches!(c.submit(&mut h, "exit 2"), Step::Exit(2)));
}

#[test]
fn syntax_errors_and_more_prompts() {
    let mut h = MemHost::new();
    let mut c = engine();
    assert_eq!(c.submit(&mut h, "if exist x ("), Step::Idle);
    assert_eq!(c.prompt(&h), "More? ");
    assert_eq!(prompt(&mut c, &mut h, "echo inside)"), "");
    assert_eq!(
        prompt(&mut c, &mut h, ") was here"),
        ") was unexpected at this time.\n"
    );
    assert_eq!(c.prompt(&h), "C:\\Users\\1001>");
    let out = batch(&mut h, "@echo off\necho before\nif (\necho after\n");
    assert!(out.starts_with("before\n"), "{out}");
    assert!(!out.contains("after"), "{out}");
    let _ = "x".to_string();
}

#[test]
fn the_token_decides_what_administrator_tools_do() {
    let mut h = MemHost::new();
    let mut c = engine();
    let out = prompt(&mut c, &mut h, "whoami /groups");
    assert!(out.contains("BUILTIN\\Administrators                 Alias            S-1-5-32-544 Group used for deny only"), "{out}");
    assert!(out.contains("Medium Mandatory Level"));
    assert!(prompt(&mut c, &mut h, "net session").contains("Access is denied."));
    assert!(prompt(&mut c, &mut h, "sfc /scannow").contains("must be an administrator"));
    h.elevated = true;
    let out = prompt(&mut c, &mut h, "whoami /groups");
    assert!(
        out.contains("Enabled group, Group owner") && out.contains("High Mandatory Level"),
        "{out}"
    );
    assert!(prompt(&mut c, &mut h, "net session").contains("There are no entries in the list."));
    assert!(prompt(&mut c, &mut h, "sfc /scannow").contains("no integrity violations"));
    assert!(prompt(&mut c, &mut h, "fsutil fsinfo drives").contains("Drives: C:\\"));
}

const WIREGUARD_CONF: &str = "[Interface]\nPrivateKey = yAnz5TF+lXXJte14tji3zlMNq+hd2rYUIgJBgB3fBmk=\nAddress = 10.8.0.2/24\n\n[Peer]\nPublicKey = xTIBA5rboUvnH4htodjb6e697QjLERt1NAB4mZqp8Dg=\nAllowedIPs = 0.0.0.0/0\nEndpoint = 10.0.2.2:51820\n";

#[test]
fn wireguard_imports_a_tunnel_and_rasdial_dials_and_hangs_up() {
    let mut host = MemHost::new();
    host.file("/users/1001/office.conf", WIREGUARD_CONF);
    host.file("/users/1001/bad.conf", "[Interface]\nPrivateKey = nope\n");
    let mut cmd = engine();
    assert!(
        prompt(&mut cmd, &mut host, "rasdial")
            .contains("No connections\nCommand completed successfully.")
    );
    // Import: silent like wireguard.exe, named after the file, dialing at once.
    let out = prompt(
        &mut cmd,
        &mut host,
        r#"wireguard /installtunnelservice "C:\Users\1001\office.conf""#,
    );
    assert!(!out.contains("Usage") && !out.contains("Unable"), "{out}");
    assert_eq!(host.vpns.len(), 1, "{out}");
    assert_eq!(
        (host.vpns[0].0.name.as_str(), host.vpns[0].1.as_str()),
        ("office", WIREGUARD_CONF)
    );
    let out = prompt(
        &mut cmd,
        &mut host,
        r"wireguard /installtunnelservice C:\Users\1001\bad.conf",
    );
    assert!(
        out.contains("Unable to load configuration: Line 2: Keys must decode to exactly 32 bytes"),
        "{out}"
    );
    let out = prompt(
        &mut cmd,
        &mut host,
        r"wireguard /installtunnelservice C:\Users\1001\missing.conf",
    );
    assert!(
        out.contains("The system cannot find the file specified."),
        "{out}"
    );
    assert!(prompt(&mut cmd, &mut host, "wireguard /bogus x").contains("Usage: wireguard.exe"));
    // rasdial: list, dial (already up after the import), hang up, dial again.
    host.vpn_connections();
    host.vpn_connections();
    assert!(
        prompt(&mut cmd, &mut host, "rasdial")
            .contains("Connected to\noffice\nCommand completed successfully.")
    );
    assert!(
        prompt(&mut cmd, &mut host, "rasdial office")
            .contains("You are already connected to office.")
    );
    assert!(
        prompt(&mut cmd, &mut host, "rasdial office /disconnect")
            .contains("Command completed successfully.")
    );
    assert!(
        prompt(&mut cmd, &mut host, "rasdial office /disconnect")
            .contains("You are not connected to office.")
    );
    let out = prompt(&mut cmd, &mut host, "rasdial OFFICE");
    assert!(out.contains("Connecting to office...\nSuccessfully connected to office.\nCommand completed successfully."), "{out}");
    // Up, the tunnel is an adapter of its own (WireGuard's: no kind
    // Windows names, no physical address).
    let out = prompt(&mut cmd, &mut host, "ipconfig /all");
    assert!(out.contains("Unknown adapter office:\n"), "{out}");
    assert!(out.contains("   Description . . . . . . . . . . . : WireGuard Tunnel\n   Physical Address. . . . . . . . . : \n   DHCP Enabled. . . . . . . . . . . : No"), "{out}");
    assert!(!prompt(&mut cmd, &mut host, "getmac").contains("office"));
    // Failures print the Remote Access error and exit with its code.
    host.vpn_outcome = (crate::host::VpnState::Failed, 868);
    prompt(&mut cmd, &mut host, "rasdial office /disconnect");
    let out = prompt(&mut cmd, &mut host, "rasdial office");
    assert!(out.contains("Remote Access error 868 - The remote connection was not made because the name of the remote access server did not resolve."), "{out}");
    assert_eq!(cmd.errorlevel, 868);
    assert!(prompt(&mut cmd, &mut host, "rasdial Home").contains("Remote Access error 623"));
    // Uninstall removes it.
    prompt(
        &mut cmd,
        &mut host,
        "wireguard /uninstalltunnelservice office",
    );
    assert!(host.vpns.is_empty());
    assert!(
        prompt(
            &mut cmd,
            &mut host,
            "wireguard /uninstalltunnelservice office"
        )
        .contains("does not exist as an installed service")
    );
}

#[test]
fn wireguard_tunnel_names_follow_its_rules() {
    assert!(super::vpn::tunnel_name("office-1.vpn"));
    for bad in ["", "has space", "x".repeat(33).as_str(), "a/b"] {
        assert!(!super::vpn::tunnel_name(bad), "{bad}");
    }
}

#[test]
fn ping_tracert_and_netstat_use_the_network_stack() {
    let mut host = MemHost::new();
    let mut cmd = engine();
    let out = prompt(&mut cmd, &mut host, "ping -n 2 127.0.0.1");
    assert!(
        out.contains("Reply from 127.0.0.1: bytes=32 time<1ms TTL=128"),
        "{out}"
    );
    assert!(out.contains("Packets: Sent = 2, Received = 2, Lost = 0 (0% loss),"));
    assert_eq!(cmd.errorlevel, 0);
    let out = prompt(&mut cmd, &mut host, "ping -n 1 10.9.9.9");
    assert!(out.contains("Request timed out."), "{out}");
    assert_eq!(cmd.errorlevel, 1);
    // Two routers, then the target; Windows' column layout.
    let out = prompt(&mut cmd, &mut host, "tracert 127.0.0.1");
    assert!(
        out.contains("\nTracing route to 127.0.0.1\nover a maximum of 30 hops:\n"),
        "{out}"
    );
    assert!(
        out.contains("  1     1 ms     1 ms     1 ms  10.0.0.1\n"),
        "{out}"
    );
    assert!(
        out.contains("  3    <1 ms    <1 ms    <1 ms  127.0.0.1\n"),
        "{out}"
    );
    assert!(out.contains("\nTrace complete."));
    let out = prompt(&mut cmd, &mut host, "tracert -h 2 10.9.9.9");
    assert!(
        out.contains("  1     *        *        *     Request timed out.\n"),
        "{out}"
    );
    host.connections = alloc::vec![
        crate::host::Connection {
            tcp: true,
            state: 2,
            local: [0, 0, 0, 0],
            local_port: 8080,
            remote: [0, 0, 0, 0],
            remote_port: 0,
            pid: 42,
            v6: None,
        },
        crate::host::Connection {
            tcp: true,
            state: 5,
            local: [10, 0, 2, 15],
            local_port: 49670,
            remote: [10, 0, 2, 2],
            remote_port: 443,
            pid: 42,
            v6: None,
        },
        crate::host::Connection {
            tcp: false,
            local_port: 5353,
            pid: 7,
            ..Default::default()
        },
        // AF_INET6: a dual-stack listener, a link-local connection and a
        // UDP socket.
        crate::host::Connection {
            tcp: true,
            state: 2,
            local_port: 8443,
            pid: 42,
            v6: Some(crate::host::Ipv6Ends::default()),
            ..Default::default()
        },
        crate::host::Connection {
            tcp: true,
            state: 5,
            local_port: 50000,
            remote_port: 7,
            pid: 42,
            v6: Some(crate::host::Ipv6Ends {
                local: [0xfe, 0x80, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x15],
                local_scope: 2,
                remote: [0xfe, 0x80, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2],
                remote_scope: 2,
            }),
            ..Default::default()
        },
        crate::host::Connection {
            tcp: false,
            local_port: 546,
            pid: 7,
            v6: Some(crate::host::Ipv6Ends::default()),
            ..Default::default()
        },
    ];
    let out = prompt(&mut cmd, &mut host, "netstat -an");
    assert!(
        out.ends_with(
            "  TCP    0.0.0.0:8080           0.0.0.0:0              LISTENING\n  TCP    10.0.2.15:49670        10.0.2.2:443           ESTABLISHED\n  TCP    [::]:8443              [::]:0                 LISTENING\n  TCP    [fe80::15%2]:50000     [fe80::2%2]:7          ESTABLISHED\n  UDP    0.0.0.0:5353           *:*\n  UDP    [::]:546               *:*\n"
        ),
        "{out}"
    );
    let out = prompt(&mut cmd, &mut host, "netstat -an -p tcpv6");
    assert!(
        out.contains("[::]:8443") && !out.contains("0.0.0.0:8080"),
        "{out}"
    );
    host.connections.retain(|c| c.v6.is_none());
    let out = prompt(&mut cmd, &mut host, "netstat -n");
    assert_eq!(
        out,
        "\nActive Connections\n\n  Proto  Local Address          Foreign Address        State\n  TCP    10.0.2.15:49670        10.0.2.2:443           ESTABLISHED\n"
    );
    let out = prompt(&mut cmd, &mut host, "netstat -ano");
    assert!(
        out.contains("  TCP    0.0.0.0:8080           0.0.0.0:0              LISTENING       42\n"),
        "{out}"
    );
    assert!(
        out.contains("  UDP    0.0.0.0:5353           *:*                                    7\n"),
        "{out}"
    );
}
#[test]
fn ipconfig_lists_ipv6_like_windows() {
    let mut host = MemHost::new();
    let mut cmd = engine();
    let out = prompt(&mut cmd, &mut host, "ipconfig");
    for line in [
        "   IPv6 Address. . . . . . . . . . . : fec0::5054:ff:fe12:3456",
        "   Link-local IPv6 Address . . . . . : fe80::5054:ff:fe12:3456%2",
        "   IPv4 Address. . . . . . . . . . . : 10.0.2.15",
        "   Default Gateway . . . . . . . . . : fe80::2%2",
        "                                       10.0.2.2",
    ] {
        assert!(out.contains(line), "{line:?} missing from:\n{out}");
    }
    let out = prompt(&mut cmd, &mut host, "ipconfig /all");
    assert!(
        out.contains(
            "   Link-local IPv6 Address . . . . . : fe80::5054:ff:fe12:3456%2(Preferred) "
        ),
        "{out}"
    );
}

#[test]
fn arp_lists_the_neighbor_table_like_windows() {
    let mut host = MemHost::new();
    let mut cmd = engine();
    let out = prompt(&mut cmd, &mut host, "arp -a");
    for line in [
        "Interface: 10.0.2.15 --- 0x2",
        "  Internet Address      Physical Address      Type",
        "  10.0.2.2              52-55-0a-00-02-02     dynamic   ",
        "  10.0.2.255            ff-ff-ff-ff-ff-ff     static    ",
    ] {
        assert!(out.contains(line), "{line:?} missing from:\n{out}");
    }
    let out = prompt(&mut cmd, &mut host, "arp -a 10.0.2.2");
    assert!(
        out.contains("10.0.2.2 ") && !out.contains("10.0.2.255"),
        "{out}"
    );
    let out = prompt(&mut cmd, &mut host, "arp -a 10.0.2.9");
    assert!(out.contains("No ARP Entries Found."), "{out}");
}

#[test]
fn netstat_prints_protocol_statistics_like_windows() {
    let mut host = MemHost::new();
    let mut cmd = engine();
    let out = prompt(&mut cmd, &mut host, "netstat -s");
    for line in [
        "IPv4 Statistics",
        "  Packets Received                   = 120",
        "  Output Requests                    = 98",
        "  Datagrams Successfully Fragmented  = 0",
        "IPv6 Statistics",
        "                            Received    Sent",
        "  Messages                  4           4         ",
        "  Echo Replies              4           0         ",
        "  Echos                     0           4         ",
        "  Router Solicitations      0           3         ",
        "  Neighbor Solicitations    0           1         ",
        "TCP Statistics for IPv4",
        "  Failed Connection Attempts          = 2",
        "  Segments Retransmitted              = 3",
        "UDP Statistics for IPv6",
        "  No Ports              = 1",
    ] {
        assert!(out.contains(line), "{line:?} missing from:\n{out}");
    }
    assert!(!out.contains("Active Connections"), "{out}");
    let out = prompt(&mut cmd, &mut host, "netstat -s -p icmpv6");
    assert!(
        out.contains("ICMPv6 Statistics") && !out.contains("IPv4"),
        "{out}"
    );
    let out = prompt(&mut cmd, &mut host, "netstat -s -p tcp");
    assert!(
        out.contains("TCP Statistics for IPv4") && out.contains("Active Connections"),
        "{out}"
    );
    assert!(!out.contains("TCP Statistics for IPv6"), "{out}");
    let out = prompt(&mut cmd, &mut host, "netstat -p icmp");
    assert!(!out.contains("ICMPv4 Statistics"), "{out}");
}

/// msiexec is Windows Installer's program in System32 (msi.dll does the
/// work), found on the path like any other program.
#[test]
fn msiexec_is_the_system32_program() {
    let mut host = MemHost::new();
    host.bytes("/compat/Windows/System32/msiexec.exe", b"MZ");
    let mut cmd = engine();
    let out = prompt(&mut cmd, &mut host, "where msiexec");
    assert!(
        out.to_ascii_lowercase()
            .contains(r"c:\windows\system32\msiexec.exe"),
        "{out}"
    );
}

#[test]
fn sc_and_net_manage_services_through_the_service_control_manager() {
    let mut h = MemHost::new();
    h.services.push(crate::host::Service {
        name: "DoorsAudio".into(),
        display: "Doors Audio".into(),
        state: 4,
        pid: 5,
        accepted: 4,
        kind: 0x10,
        start: 2,
        ..Default::default()
    });
    let mut c = engine();
    // A standard token: creating a service is refused.
    let out = prompt(&mut c, &mut h, "sc create Probe binPath= C:\\probe.exe");
    assert_eq!(out, "[SC] CreateService FAILED 5:\n\nAccess is denied.\n\n");
    assert_eq!(c.errorlevel, 5);
    h.elevated = true;
    let out = prompt(
        &mut c,
        &mut h,
        "sc create Probe binPath= \"C:\\Program Files\\probe.exe\" start= auto DisplayName= \"Probe Service\"",
    );
    assert_eq!(out, "[SC] CreateService SUCCESS\n");
    let out = prompt(&mut c, &mut h, "sc qc probe");
    assert!(
        out.starts_with("[SC] QueryServiceConfig SUCCESS\n\nSERVICE_NAME: Probe\n"),
        "{out}"
    );
    assert!(
        out.contains("        START_TYPE         : 2   AUTO_START\n"),
        "{out}"
    );
    assert!(
        out.contains("        BINARY_PATH_NAME   : C:\\Program Files\\probe.exe\n"),
        "{out}"
    );
    assert!(
        out.contains("        DISPLAY_NAME       : Probe Service\n"),
        "{out}"
    );
    // Windows' layout: empty fields keep the space after the colon, a
    // second dependency goes on its own line under the first.
    assert!(
        out.contains("        LOAD_ORDER_GROUP   : \n        TAG                : 0\n"),
        "{out}"
    );
    assert!(out.contains("        DEPENDENCIES       : \n"), "{out}");
    let out = prompt(&mut c, &mut h, "sc config Probe depend= RpcSs/Http");
    assert_eq!(out, "[SC] ChangeServiceConfig SUCCESS\n");
    let out = prompt(&mut c, &mut h, "sc qc probe");
    assert!(
        out.contains("        DEPENDENCIES       : RpcSs\n                           : Http\n"),
        "{out}"
    );
    // Depending on itself is a cycle: ERROR_CIRCULAR_DEPENDENCY, nothing
    // changed.
    let out = prompt(
        &mut c,
        &mut h,
        "sc config Probe depend= Probe start= demand",
    );
    assert_eq!(
        out,
        "[SC] ChangeServiceConfig FAILED 1059:\n\nCircular service dependency was specified.\n\n"
    );
    let out = prompt(&mut c, &mut h, "sc qc probe");
    assert!(
        out.contains("        START_TYPE         : 2   AUTO_START\n"),
        "{out}"
    );
    let out = prompt(&mut c, &mut h, "sc config Probe depend= /");
    assert_eq!(out, "[SC] ChangeServiceConfig SUCCESS\n");
    let out = prompt(&mut c, &mut h, "sc qc probe");
    assert!(out.contains("        DEPENDENCIES       : \n"), "{out}");
    let out = prompt(&mut c, &mut h, "sc query Probe");
    assert!(
        out.contains("        STATE              : 1  STOPPED\n"),
        "{out}"
    );
    let out = prompt(&mut c, &mut h, "sc stop Probe");
    assert_eq!(
        out,
        "[SC] ControlService FAILED 1062:\n\nThe service has not been started.\n\n"
    );
    // sc start passes the words after the name to ServiceMain.
    let out = prompt(&mut c, &mut h, "sc start Probe alpha \"two words\"");
    assert!(
        out.contains("        STATE              : 4  RUNNING\n"),
        "{out}"
    );
    assert!(out.contains("        PID                : 4242\n"), "{out}");
    assert_eq!(h.service_arguments, ["alpha", "two words"]);
    let out = prompt(&mut c, &mut h, "sc start Probe");
    assert_eq!(
        out,
        "[SC] StartService FAILED 1056:\n\nAn instance of the service is already running.\n\n"
    );
    // net start lists the running services by display name.
    let out = prompt(&mut c, &mut h, "net start");
    assert_eq!(
        out,
        "These Windows services are started:\n\n   Doors Audio\n   Probe Service\n\nThe command completed successfully.\n\n"
    );
    let out = prompt(&mut c, &mut h, "net stop \"Probe Service\"");
    assert_eq!(
        out,
        "The Probe Service service is stopping.\nThe Probe Service service was stopped successfully.\n\n"
    );
    let out = prompt(&mut c, &mut h, "sc description Probe \"A test service\"");
    assert_eq!(out, "[SC] ChangeServiceConfig2 SUCCESS\n");
    let out = prompt(&mut c, &mut h, "sc qdescription Probe");
    assert!(out.contains("DESCRIPTION:  A test service\n"), "{out}");
    let out = prompt(&mut c, &mut h, "sc delete Probe");
    assert_eq!(out, "[SC] DeleteService SUCCESS\n");
    let out = prompt(&mut c, &mut h, "sc query Probe");
    assert_eq!(
        out,
        "[SC] EnumQueryServicesStatus:OpenService FAILED 1060:\n\nThe specified service does not exist as an installed service.\n\n"
    );
    assert_eq!(c.errorlevel, 1060);
}

#[test]
fn eventcreate_and_wevtutil_write_and_read_the_event_log() {
    let mut h = MemHost::new();
    let mut c = engine();
    let out = prompt(
        &mut c,
        &mut h,
        "eventcreate /t error /id 100 /so DoorsProbe /d \"Probe event\"",
    );
    assert_eq!(
        out,
        "SUCCESS: An event of type 'ERROR' was created in the 'APPLICATION' log with 'DoorsProbe' as the source.\n"
    );
    let out = prompt(&mut c, &mut h, "eventcreate /t error /id 2000 /d x");
    assert_eq!(
        out,
        "ERROR: Invalid ID. ID should be in the range of 1 - 1000.\n"
    );
    let out = prompt(
        &mut c,
        &mut h,
        "eventcreate /t information /id 5 /l system /d x",
    );
    assert_eq!(out, "ERROR: Access is denied.\n");
    prompt(
        &mut c,
        &mut h,
        "eventcreate /t warning /id 7 /so Second /d \"Second event\"",
    );
    let out = prompt(
        &mut c,
        &mut h,
        "wevtutil qe Application /c:1 /rd:true /f:text",
    );
    assert!(out.starts_with("Event[0]:\n  Log Name: Application\n  Source: Second\n  Date: 2026-11-02T12:16:00.0000000Z\n  Event ID: 7\n"), "{out}");
    assert!(out.contains("  Level: Warning\n"), "{out}");
    assert!(out.contains("  Description: \nSecond event\n"), "{out}");
    let out = prompt(&mut c, &mut h, "wevtutil qe Application /c:1");
    assert!(out.starts_with("<Event xmlns='http://schemas.microsoft.com/win/2004/08/events/event'><System><Provider Name='DoorsProbe'/><EventID Qualifiers='0'>100</EventID><Version>0</Version><Level>2</Level>"), "{out}");
    assert!(out.contains("<EventRecordID>1</EventRecordID>"), "{out}");
    let out = prompt(&mut c, &mut h, "wevtutil gli Application");
    assert!(
        out.contains("numberOfLogRecords: 2\noldestRecordNumber: 1"),
        "{out}"
    );
    let out = prompt(&mut c, &mut h, "wevtutil cl Application");
    assert_eq!(out, "Failed to clear log Application. Access is denied.\n");
    h.elevated = true;
    assert_eq!(prompt(&mut c, &mut h, "wevtutil cl Application"), "");
    let out = prompt(&mut c, &mut h, "wevtutil qe Nope");
    assert!(out.starts_with("Failed to query log Nope."), "{out}");
}

#[test]
fn schtasks_creates_queries_runs_and_deletes_tasks() {
    let mut h = MemHost::new();
    let mut c = engine();
    let out = prompt(&mut c, &mut h, "schtasks");
    assert!(
        out.contains(
            "INFO: There are no scheduled tasks presently available at your access level."
        ),
        "{out}"
    );
    let out = prompt(
        &mut c,
        &mut h,
        r#"schtasks /create /tn DoorsProbe /tr "C:\Program Files\probe.exe" /sc once /st 13:30"#,
    );
    assert_eq!(
        out,
        "SUCCESS: The scheduled task \"DoorsProbe\" has successfully been created.\n"
    );
    let out = prompt(
        &mut c,
        &mut h,
        "schtasks /create /tn DoorsProbe /tr x.exe /sc once /st 13:30",
    );
    assert_eq!(
        out,
        "ERROR: Cannot create a file when that file already exists.\n"
    );
    let out = prompt(&mut c, &mut h, "schtasks /query /tn DoorsProbe");
    assert!(
        out.contains(
            "\nFolder: \\\nTaskName                                 Next Run Time          Status\n"
        ),
        "{out}"
    );
    assert!(
        out.contains("DoorsProbe                               11/2/2026 1:30:00 PM   Ready\n"),
        "{out}"
    );
    assert_eq!(
        prompt(&mut c, &mut h, "schtasks /run /tn DoorsProbe"),
        "SUCCESS: Attempted to run the scheduled task \"DoorsProbe\".\n"
    );
    assert_eq!(
        prompt(&mut c, &mut h, "schtasks /change /tn DoorsProbe /disable"),
        "SUCCESS: The parameters of scheduled task \"DoorsProbe\" have been changed.\n"
    );
    assert_eq!(
        prompt(&mut c, &mut h, "schtasks /delete /tn DoorsProbe /f"),
        "SUCCESS: The scheduled task \"DoorsProbe\" was successfully deleted.\n"
    );
    assert_eq!(
        prompt(&mut c, &mut h, "schtasks /run /tn DoorsProbe"),
        "ERROR: The system cannot find the file specified.\n"
    );
}

#[test]
fn netsh_advfirewall_rules_and_profiles() {
    use crate::host::{Host, RegValue};
    let mut h = MemHost::new();
    let mut c = engine();
    let elevation = "The requested operation requires elevation (Run as administrator).\n\n";
    let add = r#"netsh advfirewall firewall add rule name="Web Server" dir=in action=allow protocol=TCP localport=8080 program="C:\Users\1001\server.exe" profile=public"#;
    assert_eq!(prompt(&mut c, &mut h, add), elevation);
    assert_eq!(c.errorlevel, 1);
    assert_eq!(
        prompt(
            &mut c,
            &mut h,
            "netsh advfirewall set allprofiles state off"
        ),
        elevation
    );
    h.elevated = true;
    assert_eq!(prompt(&mut c, &mut h, add), "Ok.\n\n");
    assert_eq!(h.firewall_reloads, 1);
    let key = r"HKLM\System\CurrentControlSet\Services\SharedAccess\Parameters\FirewallPolicy\FirewallRules";
    let values = h.reg_values(key).unwrap();
    assert_eq!(values.len(), 1);
    assert!(values[0].0.starts_with('{') && values[0].0.len() == 38);
    assert_eq!(
        values[0].1,
        RegValue::Sz(
            r"v2.33|Action=Allow|Active=TRUE|Dir=In|Protocol=6|Profile=Public|LPort=8080|App=C:\Users\1001\server.exe|Name=Web Server|".into()
        )
    );
    let out = prompt(
        &mut c,
        &mut h,
        "netsh advfirewall firewall show rule name=\"web server\"",
    );
    assert!(out.starts_with(
        "\nRule Name:                            Web Server\n----------------------------------------------------------------------\nEnabled:                              Yes\nDirection:                            In\nProfiles:                             Public\n"
    ));
    assert!(out.contains("Protocol:                             TCP\nLocalPort:                            8080\nRemotePort:                           Any\n"));
    assert!(out.ends_with("Action:                               Allow\nOk.\n\n"));
    // Ports need TCP or UDP.
    assert_eq!(
        prompt(
            &mut c,
            &mut h,
            "netsh advfirewall firewall add rule name=X dir=in action=allow localport=80"
        ),
        "A specified value is not valid.\n\n"
    );
    assert_eq!(
        prompt(
            &mut c,
            &mut h,
            "netsh advfirewall firewall delete rule name=Nothing"
        ),
        "\nNo rules match the specified criteria.\n\n"
    );
    // Profiles.
    assert_eq!(
        prompt(
            &mut c,
            &mut h,
            "netsh advfirewall set publicprofile state off"
        ),
        "Ok.\n\n"
    );
    assert_eq!(
        prompt(
            &mut c,
            &mut h,
            "netsh advfirewall show currentprofile state"
        ),
        "\nPublic Profile Settings: \n----------------------------------------------------------------------\nState                                 OFF\nOk.\n\n"
    );
    assert_eq!(
        prompt(
            &mut c,
            &mut h,
            "netsh advfirewall set allprofiles firewallpolicy blockinbound,blockoutbound"
        ),
        "Ok.\n\n"
    );
    let out = prompt(&mut c, &mut h, "netsh advfirewall show allprofiles");
    assert!(out.contains("\nDomain Profile Settings: \n"));
    assert!(out.contains("Firewall Policy                       BlockInbound,BlockOutbound\n"));
    assert_eq!(
        prompt(
            &mut c,
            &mut h,
            "netsh advfirewall firewall delete rule name=\"Web Server\" dir=in"
        ),
        "\nDeleted 1 rule(s).\nOk.\n\n"
    );
    assert!(h.reg_values(key).unwrap().is_empty());
    assert_eq!(prompt(&mut c, &mut h, "netsh advfirewall reset"), "Ok.\n\n");
    let out = prompt(&mut c, &mut h, "netsh advfirewall show allprofiles state");
    assert_eq!(
        out.matches("State                                 ON\n")
            .count(),
        3
    );
}
