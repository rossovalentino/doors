# Ordinary session qualification

Run `cargo xtask test session --image <production-image.json>` after building
`text-qemu-x64`. The suite boots without a diagnostic token, uses QMP keyboard
events, checks actual terminal output, shuts down through the user command,
audits the raw journal/ext2/catalog independently, then boots the same disk again.

Workflows cover user-named files, create/append/read/rename/delete, directory
enumeration beyond the legacy node ceiling, copied executables, arguments,
environment/cwd, denied system writes, missing/malformed programs, repeated
launch/reap, binary-preserving redirection, and Tiny Basic editing/run/save/load.
It also exercises Ctrl-C, a 3300-line save larger than journal staging, an
unrelated write after that save, persisted theme, and a real guest reboot.
The same Tiny Basic PE is first run on Windows by the build orchestrator. Each
input, expected/observed output and actual artifact hash is retained.

`cargo xtask verify` retains the earlier cumulative P00–P10 diagnostic gates and
adds this production build/session suite. A historical 42-scenario pass alone
cannot close the reopened usability scope. Failed attempts remain in reports.
The independent critic must review current code and qualifying evidence before
the phase is closed. Finite tests do not guarantee absence of every bug.
