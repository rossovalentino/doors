# Tiny Basic application port

The interpreter is Ed Davis's unchanged `C/int-stc.c`, pinned in
`third_party/apps/tinybasic/UPSTREAM.json` with its public-domain license.
`runtime.c` supplies a narrow, original C platform using public Win32 console,
file, command-line, clock and process APIs. No Microsoft CRT is bundled.

This is a source port, not qualification of an independently distributed stock
Windows binary. The same resulting PE must run on Windows and Door. It retains
the upstream interpreter's syntax, line editor, arithmetic, loops, load and save.
The platform uses ASCII filenames, four open files, a 384 KiB coalescing heap and
bounded formatted output. These bounds are explicit application-port limits.
Upstream C parsing is not hardened for hostile BASIC source; this is a user
application isolated by the kernel, not a privileged service.

Run `tinybasic.exe` from the Door shell. Enter `help`, or enter numbered lines,
`run`, `save "example.bas"`, and `bye`. Reopen with
`tinybasic.exe example.bas`. Shell Ctrl-C cancels the child.
