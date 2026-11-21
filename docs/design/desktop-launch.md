# Interactive desktop acceleration

Separate P12 consumer candidate after the accepted sparse avatar cache. Ordinary
CLI desktop launches previously chose TCG/VGA even though the acquired Windows
host already passed actual WHPX/RAMFB native runs. Software CPU emulation remains
a substantial cost at high physical resolutions.

One shared DesktopOptions parser now selects WHPX/RAMFB by default, windowed
1280x720, with explicit `--accelerator whpx` or `--accelerator tcg`, `--resolution`
and `--headless` in any order. Duplicate/unknown/missing options fail before image
lookup or owner-volume access. Unsupported resolutions remain rejected. Headless
interactive launch uses the same default1280x720 as visible launch. Historical
regression helpers retain their explicit TCG profiles and1024x768 defaults.

The existing checked launcher still validates executable/profile/image identity,
uses private firmware state and serial/QMP identities, and propagates failures.
There is no automatic fallback, feature installation, owner-volume creation or
security-setting change. [QEMU's WHPX documentation](https://www.qemu.org/docs/master/system/whpx.html)
requires Windows Hypervisor Platform and describes legacy VGA performance limits;
this project keeps the already-tested RAMFB profile and default mitigations.

Five host tests cover defaults, all modes/backend/headless permutations,
malformed/duplicate inputs and strict selection-to-argv binding. The existing
launcher resolves the selected profile and validates the constructed accelerator,
single video device, output mode and none/SDL display before process spawn.
Wrong, duplicate, mixed, overriding or truncated arguments are rejected. That
assertion cannot leave a handed-off guest running because no child exists yet.
The private-volume desktop-default4K and
desktop-software1x tests call the same parser/launch method as CLI, then require
the actual backend, icons/text, cached drag without recapture, all three clients,
maximize/minimize/restore, durable note bytes and clean shutdown. Native validation
and independent bounded acceptance now pass. No FPS or universal host-support claim
follows. Guest artifacts are unchanged from complete build16216; only host launch
selection/tests and coordinated documentation change.

Initial source1af87d84 host19220 passed553tests and native-default24152 passed16
checks, but neither qualifies the later argv-assertion repair. Software13584's
retained failure sampled the actual scrollbar thumb at1270,250; the probe now
uses a20DIP content margin consistently across densities. All three initial
reports remain retained. The tests are headless; they do not establish that
the visible SDL frontend was exercised.

Final source1c784b9b43a1f5e7b0bc0414c09aa5db42fe438da1e2fbcdf07cd7237ff0cb16
passes host23328(555tests/33executables), default25916(16checks,WHPX/RAMFB4K)
and software19812(16checks,TCG/VGA1280x720), all with unchanged inputs. Only five
host files differ from guest build16216: desktop.rs, desktop/resolution.rs,
main.rs, session/desktop_options.rs and session/mod.rs under tools/xtask/src.
Guest sources, Cargo inputs, assets and profiles are unchanged.

Independent attributed review: reports/critics/p12-desktop-launch-critic.json,
SHA2569ede1210c78c6d2690cd93dbba7d7a1d4a0c6957ffc8658640c729157d7eedbe.
It rehashed39native references per run,26build/pair files, actual process argv,
and independently decoded both private disks. Both contain exactly20bytes
`Native density note` plus LF, object32/inode17/generation1/version3; both clean
disks hash415e6c3876dfffb771cf7ec654d7fa6be60b7ce4eb79b2157770704d99c99043.
Each session retired8processes/8threads with zero leaked frames. Owner user.raw
is absent and was not created. P12 remains partial and cumulative qualification
still depends on the original missing P00 evidence.

Latest cumulative attempt26332 (SHA70df76b203eeb46c3e9cf12b08635dd12f62e3013087f16cf7e50c8e2b1e5231) retained: original P00 report remains missing; inputs unchanged.
