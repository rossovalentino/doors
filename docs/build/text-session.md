# Run the ordinary Door text OS

From PowerShell:

```powershell
Set-Location 'C:\Users\win\Desktop\Doors OS\door-os'
cargo xtask build --profile text-qemu-x64
cargo xtask session
```

QEMU and the pinned OVMF firmware are already acquired. `session` opens a visible
QEMU window; `session --headless` is available for development. The default disk
is `out/session/user.raw`. Do not delete it to rebuild: it contains your files.
The shell's `shutdown` command flushes storage and powers off. Start `session`
again to resume that volume. `reboot` restarts inside the same QEMU instance.
`cargo xtask session stop` checks the saved emulator UUID and stops that instance.
Plain `demo` now starts this same ordinary session. Explicit diagnostic demo
profiles remain isolated regression tools.

At `/users/1001>` try:

```text
mkdir notes
write notes/hello.txt "Hello from Door"
append notes/hello.txt "Saved on the user disk"
cat notes/hello.txt
ls notes
tools wc notes/hello.txt
tools args "two words"
tools env
cp /system/bin/tools my-tools
./my-tools sum notes/hello.txt
tinybasic.exe
```

Inside Tiny Basic enter:

```text
10 for i=1 to 5
20 print i*i
30 next i
40 end
run
save "squares.bas"
bye
```

Then `cat squares.bas`, `mv squares.bas saved.bas`, and `shutdown`. After restarting,
run `tinybasic.exe saved.bas` to load and execute it. Tiny Basic's `help` describes
its interactive commands. Ctrl-C cancels a child and returns to the shell.
Theme values `0`, `1`, `2` change and persist the terminal palette.

This is an experimental development OS with one capability-authenticated local
user, a bounded development filesystem, a native utility program and a narrow
Windows source port. GUI, broad stock EXE/DLL compatibility, network access,
password login and physical hardware support are unqualified. Exact current
verification is in `project/STATUS.md`; commands above are functionality to
use within the [measured application profile](../testing/application-trials.md).
The recorded tests and limits describe the scope of that qualification.
