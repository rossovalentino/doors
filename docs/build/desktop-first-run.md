# Assets made on the host — 21 November 2026

Windows fonts, icons, cursors, sounds and wallpapers, every generated
image, mask and atlas, and the Japanese IME dictionary are not in the
repository. Before the first build (and after a Windows update), run on the
Windows 11 host:

```powershell
cargo host-assets
```

It copies each file listed in `assets/host-assets.json` from this PC (a few
are downloaded from where they were published), re-pins any whose bytes
differ, regenerates the derived assets with `cargo xtask assets build`, and
builds `assets/ime/japanese.dict` from Mozc data pinned to a commit.
Builds refuse to run until it has completed.

# Current owner storage —22October2026

Owner cleanup removed out/session/user.raw. This implementation batch does not
recreate it. To create a new empty persistent guest disk yourself, run
`cargo xtask init-user-volume` before `cargo xtask desktop`. The initializer refuses
existing/partial data. The focused Preview/Monitor tests use private disks and
require no owner initialization. Rebuilding uses
`cargo xtask test preview-monitor-current`; the latest resulting image is selected
by the desktop launcher. Historical initialization notes below are retained.

# Run and debug the current desktop

The current launcher selects one distinct supported host adapter mode by default
and requests child-only SDL pixel coordinates. It falls back to1280x720 when the
query is unavailable, ambiguous or unsupported, and records the choice. Explicit
supported modes remain available. The actual automatic headless run selected
2560x1600 with2x/200% guest density. Fractional/live scale and visible SDL quality
remain unqualified; percentages continue to describe the actual renderer.
See [viewer and title gestures](../design/native-viewer-title-gestures.md).
The current fitted-Explorer 4K sample averages 20.11 ms of coarse paint work,
maximum 30 ms. It is a different workload from the older 15.15 ms sample and does
not establish 60 FPS or input latency. Once created, keep the persistent volume as
mutable owner data; initialization is not a repeat launch step.

From PowerShell:

```powershell
Set-Location 'C:\Users\win\Desktop\Doors OS\door-os'
cargo xtask desktop
```

Press Enter at Sign in. Ctrl+Alt+G releases input, Ctrl+Alt+F toggles fullscreen.
Use guest Shut down to flush work before closing QEMU. For native4K at300%:

```powershell
cargo xtask desktop --resolution 3840x2160
```

The default uses automatic supported-mode selection with WHPX/RAMFB. Explicit software mode:

```powershell
cargo xtask desktop --accelerator tcg
```

Once initialized, `out/session/user.raw` holds persistent guest data and is
excluded from automatic cleanup. The volume created on 21 October was removed by
owner cleanup; that earlier initialization is historical. A new volume becomes
mutable when you run it. Regression commands use separate private disks.

On another checkout, initialize only if there is no owner volume:

```powershell
cargo xtask init-user-volume
```

The command refuses any existing, partial or unknown owner data. It does not repair
or erase a disk. It needs no text-session image or rebuild. Close running QEMU and
build processes first. Unsupported same-volume hard links fail without replacing
data. No host feature/security setting is changed.

To rebuild changed guest code and run the complete current desktop workflows:

```powershell
cargo xtask test desktop-current
cargo xtask desktop
```

This uses the accepted retained build layout. Other full build graphs still use
their original8GB reserve. Fast desktop-dev relinks remain explicitly unqualified.
The launcher reads the latest existing image; merely restarting QEMU does not
compile modified guest sources. Serial and launch receipts are kept with the run
under reports/runs; the owner launch pointer is out/native-desktop.json.

## Bounded first-run qualification

Valid DesktopOptions select the same retained QEMU graph before admission. The
exact desktop-default/software private consumers are enrolled too. Invalid or
unrelated command shapes keep legacy admission; no new unbounded build is admitted.
Ordinary interactive launch writes only bounded latest-session metadata to out.
The owner disk remains an intentional external mutable medium, not retained proof.

Initialization uses the exclusive workspace lease and checks QEMU/rustc quiescence
before staging and before publication. Canonical/reparse checks confine the parent;
exclusive create grants cleanup ownership only of the run-named stage. The128MiB
formatter output is synced, then independently audited for GPT mirrors/CRCs, FAT,
journal, ext2/catalog and the exact expected digest. Same-volume hard-link creation
publishes atomically without replacing an existing/racing target. Only owned
staging is removed. This synchronizes file data; no crash-durable directory-entry
guarantee is claimed. Stale unknown staging and partial owner data are preserved.

The fixed320MiB initializer reservation covers both logical names during publication
plus metadata, below the existing18GB writer guard. The owner's20GB cap still covers
out only, not target/reports. It is not a filesystem hard quota. D: is not configured.

Independent read-only review: reports/critics/p12-first-run-critic.json.

| Evidence | Run | Result |
| --- | --- | --- |
| Final host | host-1792576596573475700-10848 |674 tests /34 executables |
| Default shared launch | host-1792576447590346000-7548 |18 checks, actual WHPX/RAMFB3840x2160 |
| Software shared launch | host-1792576528479420000-15136 |18 checks, actual TCG/VGA1280x720 |
| Complete retained validation | host-1792576720130797300-26392 |Original D408 desktop evidence reopened; no new boots |
| Actual new-volume initialization | host-1792576816945398800-12436 |3 checks; new128MiB owner volume; no guest execution |

Final tooling source is e48c2d806f9dfb721fcdcd06ba28d06f28697d50f59bdc63115dd6e3c59f7187.
Native launch source44e1f411123a697d3f9af95c0b1aaa36c1d0d211d2ecf27afc947c423d6967bc
differs only by the final added cfg(test) late-busy/racer test. Removing that test
reproduces the exact original module hash. Production source and guest inputs are unchanged
across that test-only bridge. The separate host-tool bridge identifies exactly5
host source files since the accepted alignment docs; guest/assets/validator match.

Negative coverage includes existing/partial owner, unknown stage, corrupt stage,
uncertain process state, activity after staging and target publication races.
The critic independently checked both private sessions/captures, actual PID/UUID/
argv, clean retirement, private saved bytes and the actual newly created owner
while quiescent. No staging residue remained. Owner bytes are not regression input.

Visible SDL and actual owner boot remain unqualified by root. Earlier intermittent
host-tool aborts remain an open risk; no repaired cause is asserted. The original
missing P00 report still blocks cumulative verification. P12 stays partial.
