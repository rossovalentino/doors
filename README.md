# Doors

An independent x86-64 UEFI operating system written in Rust, with a Windows 11
style desktop (shown as Macrohard Doors) and a Windows personality that runs
unmodified Windows x64 and x86 programs. Its Windows-compatible system DLLs
are original Rust implementations.

## Status (November 2026)

| Phase | Scope | State |
| --- | --- | --- |
| P00-P11 | Boot, memory, SMP scheduler, native processes, VirtIO storage, journaled filesystem, services, Windows PE personality and DLLs, GUI foundations | Done (original evidence deleted, owner-confirmed) |
| P12-P14 | Native UI, text and accessibility; Windows GUI integration; desktop and visual fidelity | Done (owner-attested) |
| P15-P17 | Network, audio and media; COM, services, installers, shell and security; 32-bit programs under WOW64 | Implemented and guest-tested; cumulative gate deferred |
| **P18** | Unmodified .NET, PowerShell 7, Chromium/CEF and Electron apps | **In progress** |
| P19 | GPU acceleration (DXGI/D3D11/D3D12) | Planned |

P18 so far, in the guest:

* **Pass:** the Universal CRT probe, .NET 10 with its JIT (generics,
  exceptions, thread pool, GC, files), PowerShell 7.7 scripts and the
  interactive shell in the unified Terminal.
* **Electron 45** runs through Node's start-up into Chromium's browser set-up.
  It needs the two-CPU desktop profile. Work continues on the next start-up
  blocker; then VS Code and cefclient.
* Rendering is software-only until P19; dxgi/d3d11/d3d12/dcomp report
  "unsupported".

Known gaps: window hooks are not called; GDI paths, metafiles and world
transforms are not drawn; clipboard and OLE drag and drop work within one
process; TaskDialog has no icons or progress bars; SSPI has no packages;
DirectWrite has no text layout. Details: [project/STATUS.md](project/STATUS.md),
[P18 plan](docs/design/p18-plan-20261111.md).

## Building and running

Host: Windows 11 x64 with Visual Studio 2022 Build Tools (MSVC, Windows SDK),
the Rust toolchain from `rust-toolchain.toml`, and QEMU with WHPX enabled.

```bash
cargo host-assets
cargo xtask build --profile desktop-qemu-x64
cargo xtask init-user-volume
cargo xtask desktop
```

`cargo host-assets` runs once after cloning (and again after a Windows
update). Optional apps disk with PowerShell 7, .NET and Electron:
`cargo xtask init-apps-volume`, then `cargo xtask desktop --apps`.
More: [docs/build/desktop-first-run.md](docs/build/desktop-first-run.md).

## Assets made on your PC

The repository carries sources only. `cargo host-assets` copies the
Microsoft-owned fonts, icons, cursors, sounds and wallpapers from your own
Windows installation (a few that Windows does not keep locally are
downloaded from their published locations), downloads the pinned Mozc data
for the Japanese IME, and regenerates every derived image, mask and atlas.
[assets/host-assets.json](assets/host-assets.json) lists each file and its
source. Build output and evidence stay in the ignored `target/`, `out/` and
`reports/runs/`.

## Layout

`kernel/` kernel, `boot/` UEFI loader, `services/` system services, `fs/`
filesystems, `graphics/` compositor and window server, `desktop/` shell and
apps, `frameworks/` UI and text, `compat/` Windows personality and DLLs,
`crates/` shared libraries, `tools/xtask` build, test and evidence
orchestrator, `project/` phase ledger and status, `docs/` design and ADRs. See
[docs/repository-layout.md](docs/repository-layout.md).

## License

No license has been selected yet; all rights reserved. Third-party
components keep their own licenses ([THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md)).
