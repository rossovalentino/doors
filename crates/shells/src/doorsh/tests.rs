//! Doorsh on the in-memory host: syntax, values, pipelines, scripts,
//! safety and the cmd.exe fallback.
use super::*;
use crate::InputKind;
use crate::memhost::MemHost;
use std::string::ToString;

const HOME: &str = "C:\\Users\\1001";

fn shell() -> Doorsh {
    let env = crate::cmd::default_environment("Win", "DOORS-PC", HOME, 4);
    Doorsh::new(env, HOME, HOME)
}
/// Run `line` to idle (keys answer 'y', lines answer "Doors") and return
/// what the console shows.
fn run(sh: &mut Doorsh, h: &mut MemHost, line: &str) -> String {
    let mut step = sh.submit(h, line);
    let mut out = String::new();
    for _ in 0..1000 {
        for e in sh.take_events() {
            if let Event::Text(t) = e {
                out.push_str(&t);
            }
        }
        step = match step {
            Step::Idle | Step::Exit(_) => break,
            Step::Input(InputKind::Key) => sh.resume(h, Resume::Key('y')),
            Step::Input(InputKind::Line) => sh.resume(h, Resume::Line("Doors".to_string())),
            Step::Sleep { .. } => sh.resume(h, Resume::Tick),
            Step::Spawn(_) => sh.resume(
                h,
                Resume::Exited {
                    code: 0,
                    output: b"program output\n".to_vec(),
                },
            ),
            Step::Launch(_) => sh.resume(h, Resume::Done(true)),
        };
    }
    for e in sh.take_events() {
        if let Event::Text(t) = e {
            out.push_str(&t);
        }
    }
    out
}
fn host() -> MemHost {
    let mut h = MemHost::new();
    h.dir("/users/1001/Projects");
    h.file("/users/1001/Projects/a.txt", "alpha\nbeta\ngamma\n");
    h.file("/users/1001/Projects/big.log", &"x".repeat(5000));
    h.file("/users/1001/Projects/b.txt", "one\n");
    h.dir("/users/1001/Projects/src");
    h.file("/users/1001/Projects/src/main.dsh", "echo nested\n");
    h
}
#[test]
fn values_expressions_and_variables() {
    let (mut sh, mut h) = (shell(), host());
    assert_eq!(run(&mut sh, &mut h, "echo (1 + 2 * 3)"), "7\n");
    assert_eq!(
        run(&mut sh, &mut h, "let x = 10; echo ($x ** 2 - 1)"),
        "99\n"
    );
    assert_eq!(run(&mut sh, &mut h, "calc 2gb / 512mb"), "4\n");
    assert_eq!(run(&mut sh, &mut h, "calc 1.5kb + 512b"), "2 KB\n");
    assert_eq!(
        run(&mut sh, &mut h, "let name = \"Doors\"; echo \"Hi, $name!\""),
        "Hi, Doors!\n"
    );
    assert_eq!(
        run(
            &mut sh,
            &mut h,
            "let r = {os: \"Doors\", n: 2}; echo $r.os ${$r.n + 1}"
        ),
        "Doors 3\n"
    );
    assert_eq!(
        run(&mut sh, &mut h, "let l = [3, 1, 2]; echo $l.length $l.-1"),
        "3 2\n"
    );
    assert_eq!(
        run(
            &mut sh,
            &mut h,
            "echo (\"ab\" ++ \"cd\") ('x' in [\"x\", \"y\"])"
        ),
        "abcd true\n"
    );
    // A variable holding spaces stays one argument (no re-splitting).
    assert_eq!(
        run(&mut sh, &mut h, "let f = \"a b c\"; echo $f | count"),
        "5\n"
    );
    assert!(run(&mut sh, &mut h, "echo $missing").contains("$missing is not set"));
    assert!(run(&mut sh, &mut h, "echo (1 / 0)").contains("division by zero"));
}
#[test]
fn control_flow_functions_and_errors() {
    let (mut sh, mut h) = (shell(), host());
    let out = run(
        &mut sh,
        &mut h,
        "for i in 1..3 { if ($i == 2) { continue }; echo $i }",
    );
    assert_eq!(out, "1\n3\n");
    let out = run(
        &mut sh,
        &mut h,
        "let n = 0; while ($n < 10) { let n = ($n + 1); if ($n == 4) { break } }; echo $n",
    );
    assert_eq!(out, "4\n");
    run(&mut sh, &mut h, "fn square(x) { return ($x * $x) }");
    assert_eq!(run(&mut sh, &mut h, "square 7"), "49\n");
    run(
        &mut sh,
        &mut h,
        "fn greet(name, greeting = \"Hello\") { echo \"$greeting, $name!\" }",
    );
    assert_eq!(run(&mut sh, &mut h, "greet Doors"), "Hello, Doors!\n");
    assert_eq!(run(&mut sh, &mut h, "greet Doors Hi"), "Hi, Doors!\n");
    // Recursion.
    run(
        &mut sh,
        &mut h,
        "fn fact(n) { if ($n <= 1) { return 1 }; return ($n * (fact ($n - 1))) }",
    );
    assert_eq!(run(&mut sh, &mut h, "fact 10"), "3628800\n");
    let out = run(
        &mut sh,
        &mut h,
        "try { cat nothere.txt } catch e { echo \"caught: $e\" }",
    );
    assert!(out.starts_with("caught: nothere.txt"), "{out}");
    assert!(run(&mut sh, &mut h, "if true { echo yes } else { echo no }").contains("yes"));
    assert_eq!(
        run(&mut sh, &mut h, "true_cmd_missing || echo fallback")
            .lines()
            .last(),
        Some("fallback")
    );
    // Syntax errors point at the column.
    let out = run(&mut sh, &mut h, "echo (1 +");
    assert!(
        out.starts_with("Syntax error:") && out.contains('^'),
        "{out}"
    );
}
#[test]
fn files_tables_and_pipelines() {
    let (mut sh, mut h) = (shell(), host());
    run(&mut sh, &mut h, "cd Projects");
    assert_eq!(sh.cwd, "C:\\Users\\1001\\Projects");
    assert_eq!(sh.prompt(), "~\\Projects> ");
    let out = run(&mut sh, &mut h, "ls");
    assert!(
        out.starts_with("name") && out.contains("src") && out.contains("a.txt"),
        "{out}"
    );
    // Folders first, as File Explorer lists.
    assert!(out.find("src").unwrap() < out.find("a.txt").unwrap());
    assert_eq!(
        run(&mut sh, &mut h, "ls | where type == file | count"),
        "3\n"
    );
    assert_eq!(
        run(&mut sh, &mut h, "ls | where size > 1kb | get name | first"),
        "big.log\n"
    );
    assert_eq!(run(&mut sh, &mut h, "ls *.txt | count"), "2\n");
    assert_eq!(run(&mut sh, &mut h, "cat a.txt | grep ^b"), "beta\n");
    assert_eq!(
        run(&mut sh, &mut h, "cat a.txt | lines | sort -r | first"),
        "gamma\n"
    );
    run(
        &mut sh,
        &mut h,
        "ls | where type == file | select name size | to csv > files.csv",
    );
    let csv = h.text("/users/1001/Projects/files.csv").unwrap();
    assert!(
        csv.starts_with("name,size\n") && csv.contains("big.log,4.9 KB"),
        "{csv}"
    );
    assert_eq!(
        run(&mut sh, &mut h, "cat files.csv | from csv | count"),
        "3\n"
    );
    assert_eq!(
        run(&mut sh, &mut h, "echo {\"a\": [1, 2]} | to json"),
        "{\"a\": [1, 2]}\n"
    );
    assert_eq!(
        run(&mut sh, &mut h, "echo '{\"x\": 5}' | from json | get x"),
        "5\n"
    );
    // Text files end with a line break (echo > file, then append).
    run(
        &mut sh,
        &mut h,
        "echo native-proof > proof.txt; append proof.txt second-line",
    );
    assert_eq!(
        h.text("/users/1001/Projects/proof.txt").as_deref(),
        Some("native-proof\nsecond-line\n")
    );
    run(&mut sh, &mut h, "mkdir deep\\er\\folder");
    assert!(h.text("/users/1001/Projects/deep/er/folder").is_some());
    run(&mut sh, &mut h, "cp a.txt deep; mv b.txt c.txt");
    assert!(
        h.text("/users/1001/Projects/deep/a.txt").is_some()
            && h.text("/users/1001/Projects/c.txt").is_some()
    );
    assert_eq!(
        run(&mut sh, &mut h, "echo copy-{1..3}"),
        "copy-1 copy-2 copy-3\n"
    );
    assert_eq!(
        run(&mut sh, &mut h, "find . *.dsh | get path"),
        "C:\\Users\\1001\\Projects\\src\\main.dsh\n"
    );
}
#[test]
fn scripts_run_with_arguments_and_source_shares_scope() {
    let (mut sh, mut h) = (shell(), host());
    h.file("/users/1001/greet.dsh", "# a script\nfn twice(x) { return ($x * 2) }\nlet who = $args.0\necho \"hello $who\" (twice 21)\n");
    assert_eq!(
        run(&mut sh, &mut h, "run greet.dsh Doors"),
        "hello Doors 42\n"
    );
    // run keeps its variables to itself; source shares them.
    assert!(run(&mut sh, &mut h, "echo $who").contains("not set"));
    run(&mut sh, &mut h, "source greet.dsh x");
    assert_eq!(run(&mut sh, &mut h, "echo $who"), "x\n");
    h.file("/users/1001/bad.dsh", "echo ok\nif (1 { }\n");
    assert!(run(&mut sh, &mut h, "run bad.dsh").contains("bad.dsh, line 2"));
}
#[test]
fn deleting_asks_and_protects_system_locations() {
    let (mut sh, mut h) = (shell(), host());
    run(&mut sh, &mut h, "cd Projects");
    // Several items ask first (the test answers y).
    let out = run(&mut sh, &mut h, "rm a.txt b.txt");
    assert!(out.contains("Delete these 2 items permanently?"), "{out}");
    assert!(h.text("/users/1001/Projects/a.txt").is_none());
    assert!(run(&mut sh, &mut h, "rm src").contains("use rm -r"));
    run(&mut sh, &mut h, "rm -r src");
    assert!(h.text("/users/1001/Projects/src/main.dsh").is_none());
    assert!(run(&mut sh, &mut h, "rm -rf C:\\Windows").contains("protected location"));
    assert!(run(&mut sh, &mut h, "rm -rf ~").contains("protected location"));
    assert!(h.text("/compat/Windows/System32").is_some());
}
#[test]
fn windows_tools_programs_and_suggestions() {
    let (mut sh, mut h) = (shell(), host());
    // System32 tools run through cmd.exe and flow into the pipeline.
    let out = run(&mut sh, &mut h, "ipconfig | grep IPv4");
    assert!(out.contains("10.0.2.15"), "{out}");
    assert_eq!(run(&mut sh, &mut h, "hostname"), "DOORS-PC\n");
    // Windows names map onto Doorsh commands.
    assert!(run(&mut sh, &mut h, "dir").starts_with("name"));
    let out = run(&mut sh, &mut h, "lss");
    assert!(out.contains("Did you mean 'ls'?"), "{out}");
    assert_eq!(sh.status, 9009);
    assert!(
        run(&mut sh, &mut h, "explain 'ls | where type == file'")
            .contains("where (Doorsh command)")
    );
    assert!(run(&mut sh, &mut h, "help").contains("Text and data:"));
    assert!(run(&mut sh, &mut h, "help grep").contains("Usage:   grep pattern"));
    // History: !! repeats the last line.
    run(&mut sh, &mut h, "echo again");
    assert!(run(&mut sh, &mut h, "!!").ends_with("again\n"));
    let names = sh.complete(&mut h, "so");
    assert!(names.contains(&"sort".to_string()) && names.contains(&"sort-by".to_string()));
    run(&mut sh, &mut h, "cd Projects");
    assert_eq!(sh.complete(&mut h, "cat a"), ["a.txt"]);
}

#[test]
fn netsh_runs_through_the_cmd_fallback() {
    let (mut sh, mut h) = (shell(), host());
    h.elevated = true;
    let out = run(
        &mut sh,
        &mut h,
        "netsh advfirewall firewall add rule name=DoorsProbe dir=in action=block protocol=tcp localport=9",
    );
    assert!(out.contains("Ok."), "{out}");
    let out = run(
        &mut sh,
        &mut h,
        "netsh advfirewall firewall show rule name=DoorsProbe",
    );
    assert!(
        out.contains("Rule Name:                            DoorsProbe"),
        "{out}"
    );
    assert!(
        out.contains("Action:                               Block"),
        "{out}"
    );
    let out = run(
        &mut sh,
        &mut h,
        "netsh advfirewall firewall delete rule name=DoorsProbe",
    );
    assert!(out.contains("Deleted 1 rule(s)."), "{out}");
    let out = run(&mut sh, &mut h, "netsh advfirewall show currentprofile");
    assert!(out.contains("BlockInbound,AllowOutbound"), "{out}");
}
