//! Doorsh's built-in reference: `help`, `help <command>`, usage lines for
//! errors, "did you mean" suggestions and `explain`.
use super::Doorsh;
use super::parse::{Cond, Iter, Joint, Part, Stmt, Word};
use alloc::{format, string::String, vec::Vec};

/// (command, usage, one-line description, example).
const COMMANDS: &[(&str, &str, &str, &str)] = &[
    // Navigation
    (
        "cd",
        "cd [folder | - | ~ | bookmark]",
        "Change the current folder (- goes back).",
        "cd ~\\Documents",
    ),
    ("pwd", "pwd", "Show the current folder.", "pwd"),
    (
        "ls",
        "ls [path...] [-a] [-l]",
        "List a folder as a table (name, type, size, modified).",
        "ls | where size > 1mb | sort-by size -r",
    ),
    (
        "tree",
        "tree [path] [-f] [-depth n]",
        "Draw the folder tree (-f includes files).",
        "tree src -depth 2",
    ),
    (
        "pushd",
        "pushd folder",
        "Go to a folder, remembering the current one.",
        "pushd C:\\Windows",
    ),
    (
        "popd",
        "popd",
        "Return to the folder pushd remembered.",
        "popd",
    ),
    ("dirs", "dirs", "List the remembered folders.", "dirs"),
    (
        "bookmark",
        "bookmark [name [folder]] [-d]",
        "Name a folder (the current one by default); list bookmarks.",
        "bookmark work ~\\Projects",
    ),
    ("go", "go name", "Go to a bookmarked folder.", "go work"),
    // Files
    (
        "cat",
        "cat file...",
        "Show the text of files.",
        "cat notes.txt",
    ),
    (
        "head",
        "head [file] [-n count]",
        "The first lines (10 by default).",
        "cat log.txt | head -n 5",
    ),
    (
        "tail",
        "tail [file] [-n count]",
        "The last lines.",
        "tail log.txt -n 20",
    ),
    (
        "wc",
        "wc [file]",
        "Count lines, words, characters and bytes.",
        "cat report.txt | wc",
    ),
    (
        "stat",
        "stat path",
        "Details of a file or folder.",
        "stat setup.exe",
    ),
    (
        "du",
        "du [folder]",
        "Total size, files and folders under a folder.",
        "du ~\\Downloads",
    ),
    ("df", "df", "Size and free space of the drives.", "df"),
    (
        "find",
        "find [folder] [pattern] [-type file|folder] [-depth n]",
        "Find files and folders by name, recursively.",
        "find ~ *.dsh",
    ),
    (
        "mkdir",
        "mkdir folder...",
        "Create folders (and any missing parents).",
        "mkdir projects\\new\\src",
    ),
    (
        "touch",
        "touch file...",
        "Create empty files or update their time.",
        "touch todo.txt",
    ),
    (
        "cp",
        "cp source... destination [-r]",
        "Copy files (-r copies folders).",
        "cp *.txt backup",
    ),
    (
        "mv",
        "mv source... destination",
        "Move or rename files and folders.",
        "mv draft.txt final.txt",
    ),
    (
        "rename",
        "rename old new",
        "Rename a file or folder.",
        "rename a.txt b.txt",
    ),
    (
        "rm",
        "rm path... [-r] [-f]",
        "Delete files (-r folders). Several items or a folder tree ask first; system locations are refused.",
        "rm old.log",
    ),
    (
        "rmdir",
        "rmdir folder",
        "Delete an empty folder.",
        "rmdir empty",
    ),
    (
        "hash",
        "hash file... [-a SHA256|SHA1|MD5]",
        "Checksums of files.",
        "hash setup.exe",
    ),
    (
        "diff",
        "diff file1 file2",
        "Line differences between two text files.",
        "diff old.txt new.txt",
    ),
    (
        "save",
        "... | save file [-a]",
        "Write the piped value to a file (-a appends).",
        "ls | to csv | save files.csv",
    ),
    (
        "append",
        "... | append file  |  append file text...",
        "Append the piped value (or the words) to a file.",
        "date | append log.txt",
    ),
    (
        "write",
        "write file text...",
        "Write words to a file, replacing it.",
        "write note.txt remember the milk",
    ),
    (
        "open",
        "open path",
        "Open a file or folder with its default app.",
        "open report.pdf",
    ),
    (
        "explorer",
        "explorer [folder]",
        "Open a folder in File Explorer.",
        "explorer .",
    ),
    (
        "edit",
        "edit file",
        "Open a file in Notes.",
        "edit script.dsh",
    ),
    (
        "settings",
        "settings [page or legacy name]",
        "Open a Settings page.",
        "settings display",
    ),
    // Text and data
    (
        "echo",
        "echo value... [-n]",
        "Output values (-n: no new line).",
        "echo \"Hello, $USERNAME\"",
    ),
    ("print", "print value...", "Same as echo.", "print (1 + 2)"),
    (
        "grep",
        "grep pattern [file...] [-i] [-v] [-n]",
        "Keep lines (or table rows) matching a pattern (* ? ^ $).",
        "cat log.txt | grep error -n",
    ),
    (
        "lines",
        "... | lines",
        "Split text into a list of lines.",
        "cat list.txt | lines | count",
    ),
    (
        "words",
        "... | words",
        "Split text into words.",
        "cat essay.txt | words | count",
    ),
    (
        "chars",
        "... | chars",
        "Split text into characters.",
        "echo abc | chars",
    ),
    (
        "split",
        "... | split [separator]",
        "Split text at a separator (, by default).",
        "echo a,b,c | split",
    ),
    (
        "join",
        "... | join [separator]",
        "Join a list into text.",
        "ls | get name | join \", \"",
    ),
    (
        "replace",
        "... | replace old new",
        "Replace text.",
        "cat a.txt | replace foo bar",
    ),
    (
        "trim",
        "... | trim",
        "Remove surrounding spaces.",
        "echo \"  x  \" | trim",
    ),
    (
        "upper",
        "... | upper",
        "Upper-case text.",
        "echo doors | upper",
    ),
    (
        "lower",
        "... | lower",
        "Lower-case text.",
        "echo DOORS | lower",
    ),
    (
        "sort",
        "... | sort [-r]",
        "Sort a list.",
        "cat names.txt | lines | sort",
    ),
    (
        "sort-by",
        "... | sort-by field [-r]",
        "Sort table rows by a field.",
        "ls | sort-by modified -r",
    ),
    (
        "uniq",
        "... | uniq",
        "Drop repeated items.",
        "cat tags.txt | lines | uniq",
    ),
    (
        "reverse",
        "... | reverse",
        "Reverse a list.",
        "seq 1 5 | reverse",
    ),
    (
        "where",
        "... | where condition",
        "Keep rows where the condition holds (fields by name).",
        "ls | where type == file && size > 10kb",
    ),
    (
        "select",
        "... | select field...",
        "Keep only these fields.",
        "ps | select name memory",
    ),
    (
        "reject",
        "... | reject field...",
        "Drop these fields.",
        "ls | reject modified",
    ),
    (
        "get",
        "... | get field[.field]",
        "A field (or a column of a table).",
        "sysinfo | get os",
    ),
    (
        "first",
        "... | first [n]",
        "The first item (or n items).",
        "ls | first 3",
    ),
    (
        "last",
        "... | last [n]",
        "The last item (or n items).",
        "history | last 5",
    ),
    (
        "take",
        "... | take n",
        "The first n items.",
        "seq 1 100 | take 10",
    ),
    (
        "skip",
        "... | skip n",
        "All but the first n items.",
        "cat data.csv | lines | skip 1",
    ),
    (
        "count",
        "... | count",
        "How many items (or characters).",
        "ls *.txt | count",
    ),
    (
        "sum",
        "... | sum [field]",
        "Add the items (or a field).",
        "ls | sum size",
    ),
    ("avg", "... | avg [field]", "The average.", "ls | avg size"),
    (
        "min",
        "... | min [field]",
        "The smallest item.",
        "ls | min size",
    ),
    (
        "max",
        "... | max [field]",
        "The largest item.",
        "ps | max memory",
    ),
    (
        "enumerate",
        "... | enumerate",
        "Number the items.",
        "cat todo.txt | lines | enumerate",
    ),
    (
        "flatten",
        "... | flatten",
        "Merge nested lists.",
        "[[1, 2], [3]] | flatten",
    ),
    (
        "group-by",
        "... | group-by field",
        "Group rows by a field, with counts.",
        "ls | group-by type",
    ),
    (
        "columns",
        "... | columns",
        "The field names of a table or record.",
        "ps | columns",
    ),
    (
        "compact",
        "... | compact",
        "Drop empty items.",
        "cat a.txt | lines | compact",
    ),
    (
        "default",
        "... | default value",
        "A value to use when the input is empty.",
        "$env.EDITOR | default notes",
    ),
    (
        "is-empty",
        "... | is-empty",
        "True when the input is empty.",
        "ls | is-empty",
    ),
    (
        "describe",
        "... | describe",
        "The type of a value.",
        "ls | describe",
    ),
    (
        "from",
        "... | from json|csv|lines",
        "Read structured data from text.",
        "cat data.json | from json",
    ),
    (
        "to",
        "... | to json|csv|text",
        "Write a value as text.",
        "ps | to json | save ps.json",
    ),
    (
        "format",
        "... | format template",
        "Fill {field} placeholders from each row.",
        "ls | format \"{name} is {size}\"",
    ),
    (
        "each-line",
        "... | each-line template",
        "Make text from each item ({} is the item).",
        "seq 1 3 | each-line \"item {}\"",
    ),
    (
        "base64",
        "base64 [text] [-d]",
        "Encode (or decode with -d) base64.",
        "echo hello | base64",
    ),
    ("uuid", "uuid", "A new random identifier.", "uuid"),
    (
        "random",
        "random [min] max",
        "A random whole number.",
        "random 1 6",
    ),
    ("seq", "seq [first] last", "A list of numbers.", "seq 1 10"),
    (
        "calc",
        "calc expression",
        "Calculate (sizes and durations allowed).",
        "calc 2gb / 512mb",
    ),
    // System
    (
        "ps",
        "ps",
        "Running processes.",
        "ps | sort-by memory -r | first 5",
    ),
    (
        "kill",
        "kill pid|name... [-f]",
        "End your own processes.",
        "kill notepad",
    ),
    ("sysinfo", "sysinfo", "About this PC.", "sysinfo"),
    ("uptime", "uptime", "Time since start-up.", "uptime"),
    (
        "whoami",
        "whoami [-all]",
        "The signed-in user.",
        "whoami -all",
    ),
    ("hostname", "hostname", "This PC's name.", "hostname"),
    (
        "env",
        "env",
        "Environment variables.",
        "env | where name like *path*",
    ),
    (
        "set-env",
        "set-env name value [-persist]",
        "Set an environment variable (-persist keeps it).",
        "set-env EDITOR notes -persist",
    ),
    (
        "unset",
        "unset name...",
        "Remove a variable or function.",
        "unset tmp",
    ),
    ("vars", "vars", "Your variables.", "vars"),
    ("fns", "fns", "Your functions.", "fns"),
    ("which", "which name", "What a name runs.", "which ls"),
    (
        "alias",
        "alias [name [= command]]",
        "Name a command line; list aliases.",
        "alias gs = git status",
    ),
    ("unalias", "unalias name", "Remove an alias.", "unalias gs"),
    (
        "history",
        "history [filter]",
        "Commands you ran (!! repeats the last, !text the last starting with text).",
        "history cd",
    ),
    (
        "time",
        "time command...",
        "Run a command and show how long it took.",
        "time find ~ *.txt",
    ),
    (
        "sleep",
        "sleep duration",
        "Wait (500ms, 2s, 1min).",
        "sleep 2s",
    ),
    ("pause", "pause", "Wait for a key.", "pause"),
    (
        "input",
        "input [prompt]",
        "Read a line from the keyboard.",
        "let name = (input \"Name: \")",
    ),
    (
        "confirm",
        "confirm [question]",
        "Ask yes or no.",
        "if (confirm \"Delete?\") { rm a.txt }",
    ),
    (
        "date",
        "date [-r]",
        "The date and time (-r as a record).",
        "date -r | get weekday",
    ),
    ("clear", "clear", "Clear the console.", "clear"),
    ("title", "title text", "Set the tab's title.", "title Build"),
    (
        "clip",
        "clip [text]",
        "Copy text (or the piped value) to the clipboard.",
        "pwd | clip",
    ),
    (
        "paste",
        "paste",
        "The clipboard's text.",
        "paste | lines | count",
    ),
    ("exit", "exit [code]", "Close this tab.", "exit"),
    (
        "version",
        "version",
        "Doorsh and Doors versions.",
        "version",
    ),
    ("help", "help [command]", "This help.", "help where"),
    (
        "explain",
        "explain command line",
        "Show how Doorsh reads a line.",
        "explain ls | where size > 1mb",
    ),
    (
        "run",
        "run script.dsh [args...]",
        "Run a Doorsh script ($args holds its arguments).",
        "run backup.dsh ~\\Documents",
    ),
    (
        "source",
        "source script.dsh",
        "Run a script in this session (keeps its variables and functions).",
        "source profile.dsh",
    ),
    // Network
    (
        "ping",
        "ping host [-n count]",
        "Check that a computer answers.",
        "ping example.com -n 2",
    ),
    (
        "resolve",
        "resolve name",
        "The address of a name.",
        "resolve example.com",
    ),
    (
        "ip",
        "ip",
        "Network adapters and addresses.",
        "ip | select adapter address",
    ),
];
const GROUPS: &[(&str, &[&str])] = &[
    (
        "Navigation",
        &[
            "cd", "pwd", "ls", "tree", "pushd", "popd", "dirs", "bookmark", "go",
        ],
    ),
    (
        "Files",
        &[
            "cat", "head", "tail", "wc", "stat", "du", "df", "find", "mkdir", "touch", "cp", "mv",
            "rename", "rm", "rmdir", "hash", "diff", "save", "append", "write", "open", "explorer",
            "edit", "settings",
        ],
    ),
    (
        "Text and data",
        &[
            "echo",
            "grep",
            "lines",
            "words",
            "chars",
            "split",
            "join",
            "replace",
            "trim",
            "upper",
            "lower",
            "sort",
            "sort-by",
            "uniq",
            "reverse",
            "where",
            "select",
            "reject",
            "get",
            "first",
            "last",
            "take",
            "skip",
            "count",
            "sum",
            "avg",
            "min",
            "max",
            "enumerate",
            "flatten",
            "group-by",
            "columns",
            "compact",
            "default",
            "is-empty",
            "describe",
            "from",
            "to",
            "format",
            "each-line",
            "base64",
            "uuid",
            "random",
            "seq",
            "calc",
        ],
    ),
    (
        "System",
        &[
            "ps", "kill", "sysinfo", "uptime", "whoami", "hostname", "env", "set-env", "unset",
            "vars", "fns", "which", "alias", "unalias", "history", "time", "sleep", "pause",
            "input", "confirm", "date", "clear", "title", "clip", "paste", "exit", "version",
            "help", "explain", "run", "source",
        ],
    ),
    ("Network", &["ping", "resolve", "ip"]),
];
pub fn usage(cmd: &str) -> &'static str {
    COMMANDS.iter().find(|c| c.0 == cmd).map_or("help", |c| c.1)
}
pub fn overview() -> String {
    let mut out = String::from(
        "Doorsh, the Doors shell. Commands pass tables and values along pipelines:\n  ls | where size > 1mb | sort-by size -r | first 5\n\nSyntax: let x = (expr)   if (cond) { } else { }   for f in (ls) { }   while (cond) { }\n        fn name(a, b = 1) { }   try { } catch e { }   $x  $(cmd)  ${expr}  \"text $x\"\n        a | b   a && b   a || b   > file   >> file   < file   # comment\n\n",
    );
    for (group, names) in GROUPS {
        out.push_str(group);
        out.push_str(":\n");
        let mut line = String::from("  ");
        for n in *names {
            if line.len() + n.len() + 2 > 78 {
                out.push_str(line.trim_end());
                out.push('\n');
                line = String::from("  ");
            }
            line.push_str(n);
            line.push_str("  ");
        }
        out.push_str(line.trim_end());
        out.push_str("\n\n");
    }
    out.push_str("Any other name runs the Windows tool or program of that name (ipconfig, notepad, setup.exe).\nWindows names work too: dir, type, copy, move, del, md, cls. Type 'help <command>' for details.\n");
    out
}
pub fn topic(cmd: &str) -> String {
    match COMMANDS.iter().find(|c| c.0 == cmd) {
        Some((_, usage, what, example)) => {
            format!("{what}\n\nUsage:   {usage}\nExample: {example}\n")
        }
        None => match cmd {
            "syntax" | "let" | "if" | "for" | "while" | "fn" | "try" => String::from(
                "let total = (ls | count)            variables hold any value\nif ($total > 10) { echo big } else { echo small }\nfor f in (ls *.txt) { echo $f.name }\nwhile ($n < 3) { let n = ($n + 1) }\nfn greet(name, greeting = \"Hello\") { echo \"$greeting, $name!\" }\ntry { cat missing.txt } catch e { echo \"failed: $e\" }\nScripts: save lines in name.dsh and run them with 'run name.dsh' or './name.dsh'.\n",
            ),
            _ => format!("No help for '{cmd}'. Type 'help' for the list of commands.\n"),
        },
    }
}
/// Edit distance (insertions, deletions, substitutions) for suggestions.
pub fn distance(a: &str, b: &str) -> usize {
    let (a, b): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.iter().enumerate() {
        let mut cur = alloc::vec![i + 1];
        for (j, cb) in b.iter().enumerate() {
            cur.push(
                (prev[j] + usize::from(ca != cb))
                    .min(prev[j + 1] + 1)
                    .min(cur[j] + 1),
            );
        }
        prev = cur;
    }
    prev[b.len()]
}
fn word(w: &Word) -> String {
    w.parts
        .iter()
        .map(|p| match p {
            Part::Bare(s) => s.clone(),
            Part::Quoted(s) => format!("'{s}'"),
            Part::Var(n, path) => format!(
                "${n}{}",
                path.iter().map(|p| format!(".{p}")).collect::<String>()
            ),
            Part::Sub(_) => String::from("$(...)"),
            Part::Expr(_) => String::from("(...)"),
        })
        .collect()
}
/// How a statement is read, one line per part.
pub fn explain(sh: &Doorsh, s: &Stmt, depth: usize, out: &mut String) {
    let pad = "  ".repeat(depth);
    let kind = |name: &str| -> &'static str {
        if sh.has_function(name) {
            "your function"
        } else if sh.aliases.contains_key(name) {
            "alias"
        } else if super::builtins::is_builtin(name) {
            "Doorsh command"
        } else {
            "Windows tool or program (through cmd.exe)"
        }
    };
    match s {
        Stmt::Chain(items) => {
            for (joint, p) in items {
                let lead = match joint {
                    Joint::First => "run",
                    Joint::And => "then, if that succeeded,",
                    Joint::Or => "otherwise, if that failed,",
                };
                out.push_str(&format!("{pad}{lead}:\n"));
                for (i, c) in p.commands.iter().enumerate() {
                    let name = c.words.first().map(word).unwrap_or_default();
                    let rest: Vec<String> = c.words.iter().skip(1).map(word).collect();
                    let arrow = if i == 0 { "  " } else { "| " };
                    out.push_str(&format!(
                        "{pad}  {arrow}{name} ({}){}{}\n",
                        kind(&name),
                        if rest.is_empty() { "" } else { " with " },
                        rest.join(", ")
                    ));
                    for r in &c.redirects {
                        out.push_str(&format!(
                            "{pad}     redirect {:?} {}\n",
                            r.kind,
                            word(&r.target)
                        ));
                    }
                }
            }
        }
        Stmt::Let(n, _) => out.push_str(&format!(
            "{pad}set variable ${n} to the value of an expression\n"
        )),
        Stmt::SetEnv(n, _) => out.push_str(&format!("{pad}set environment variable {n}\n")),
        Stmt::If(arms, otherwise) => {
            for (i, (c, body)) in arms.iter().enumerate() {
                let cond = match c {
                    Cond::Expr(_) => "an expression is true",
                    Cond::Run(_) => "a command succeeds",
                };
                out.push_str(&format!(
                    "{pad}{} {cond}:\n",
                    if i == 0 { "if" } else { "else if" }
                ));
                for s in body {
                    explain(sh, s, depth + 1, out);
                }
            }
            if let Some(body) = otherwise {
                out.push_str(&format!("{pad}else:\n"));
                for s in body {
                    explain(sh, s, depth + 1, out);
                }
            }
        }
        Stmt::For(v, iter, body) => {
            let source = match iter {
                Iter::Expr(_) => "a value",
                Iter::Run(_) => "a command's output",
            };
            out.push_str(&format!("{pad}for each item of {source}, as ${v}:\n"));
            for s in body {
                explain(sh, s, depth + 1, out);
            }
        }
        Stmt::While(_, body) | Stmt::Loop(body) => {
            out.push_str(&format!("{pad}repeat while the condition holds:\n"));
            for s in body {
                explain(sh, s, depth + 1, out);
            }
        }
        Stmt::Fn(f) => out.push_str(&format!(
            "{pad}define function {}({})\n",
            f.name,
            f.params
                .iter()
                .map(|p| p.0.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        )),
        Stmt::Return(_) => out.push_str(&format!("{pad}return a value\n")),
        Stmt::Break => out.push_str(&format!("{pad}leave the loop\n")),
        Stmt::Continue => out.push_str(&format!("{pad}next round of the loop\n")),
        Stmt::Try(body, var, _) => {
            out.push_str(&format!("{pad}try:\n"));
            for s in body {
                explain(sh, s, depth + 1, out);
            }
            out.push_str(&format!(
                "{pad}on failure{}: run the catch block\n",
                var.as_ref()
                    .map(|v| format!(" (message in ${v})"))
                    .unwrap_or_default()
            ));
        }
    }
}
