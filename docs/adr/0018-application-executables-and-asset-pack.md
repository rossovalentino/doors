# ADR 0018: Per-application executables and the shared application asset pack

Status: implemented on host and guest-compiled (W9 foundation); guest boot and
desktop scenarios not yet run against it.

## Context

`door-workspace` was one native executable that ran File Explorer, Notes,
Settings, Terminal and the Start search host, selected by launch mode. Fonts,
icon masks/bitmaps, branding and wallpaper thumbnails were compiled into it with
`include_bytes!`, and again into `door-preview` and `door-monitor`. The single
binary reached 16.46 MB against the 16 MiB native loader limit
(`door_initfs::MAX_NATIVE_FILE`); Preview and Monitor were 4.9 and 4.6 MB, about
15 MB of each being duplicated read-only data. Every running desktop process held
its own copy of those resources, and the desktop initfs used about 58.5 of its
64 MiB archive budget.

## Decision

1. **One executable per application, one shared library.** The `door-workspace`
   package builds `door-files`, `door-notes`, `door-settings`, `door-terminal` and
   `door-search` (`src/*_main.rs`). Each root implements the small `native::Hosted`
   adapter over its application type and calls `door_workspace::runtime::run`;
   all behavior stays in the library, so host tests are unchanged.
2. **Application identity is unchanged.** No manifest is added. Init still
   launches every one of them as manifest `WORKSPACE` (12) with launch modes
   DESKTOP (Files), SESSION (Notes), SETTINGS, TERMINAL and SEARCH_HOST; the
   kernel chooses the image from the authenticated initfs by that mode
   (`WORKSPACE_IMAGES` in `kernel/src/process/platform.rs`). Policy, uid,
   window-server app classes, taskbar grouping and pins, the Start catalog,
   Task Manager rows, shutdown accounting, multi-window, tab tear-off,
   OPEN_DOCUMENT routing and Recent all key on (manifest, mode) and need no change.
   Task Manager now shows the per-application image name. The 12-process and
   8-window-owner limits are not raised: every application already was its own
   process.
3. **Runtime assets.** `profiles/build/app-assets.txt` lists the `assets/`
   files desktop applications load (1447 keys, 12.84 MB of data). The build
   generates `DOORAST1` (`door_initfs::app_assets`) from exactly those files into
   the build directory and packs it as `system/app-assets.pack`, initfs class
   `resource-pack` (validated as a pack, never accepted as an executable). The
   pack is reproduced in the independent second build pass and compared like
   every application artifact; it is not checked in.
4. **One copy in memory.** In DESKTOP mode the kernel parses the pack once from
   the authenticated archive. On the first launch of a painting desktop
   application it copies the bytes into one section and registers it as an
   executive *resource section*: the executive holds a pinning reference, so the
   section outlives its grantees, and grants carry exactly QUERY|READ|MAP (no
   WRITE, DUPLICATE or TRANSFER). Workspace (except the search host), Preview and
   Monitor receive it in StartupV2 inherited slot 7 (`inherited::APP_ASSETS`) and
   map it read-only at `0x7E00_0000`, the top 32 MiB of the user section window.
   At platform shutdown, after every process has been reaped, the pin is released
   and the section retires through the normal section path, so the existing
   zero-leak shutdown assertions still hold.
5. **Lookup.** `door_workspace::asset!(path)` replaces `include_bytes!`. The
   key is the path after its last `assets/`; lookup is a binary search returning
   `&'static [u8]`, empty when missing (painters already treat wrongly sized
   input as absent). Fonts come from the pack (`FontAtlas::from_packed` and
   `FontAtlas::monospace_from`). On the host the same keys are read from the
   repository and cached, and a key absent from the manifest fails the test, so
   a pack omission cannot pass host tests and then render blank on a guest.

## Consequences

- Guest release sizes (x86_64-unknown-none, release, xtask flags): door-files
  1.71 MB, door-notes 0.69 MB, door-settings 1.03 MB, door-terminal 0.71 MB,
  door-search 0.15 MB (formerly one 16.46 MB executable); door-preview 0.95 MB
  (was 4.92 MB), door-monitor 0.65 MB (was 4.62 MB). Every application now has
  headroom under the loader limit and can grow independently.
- Initfs: five small executables plus one 12,918,547-byte pack replace the
  16.46 MB executable and the duplicated resources in Preview and Monitor, about
  7 MB less archive in all.
- Memory: one ~12.9 MB physical copy shared by all desktop applications instead
  of up to ~15 MB of resource data per process.
- Window surfaces lose the top 32 MiB of the user section window: the per-process
  surface pool is 48 x 4 MiB (192 MiB). The largest single section is unchanged
  (32 MiB).
- The asset section counts against `MAX_SECTIONS` (32) once, for the boot.
- A new executable must be added to `WORKSPACE_IMAGE_PATHS`, the xtask GUI list
  and both desktop initfs profiles together; the kernel refuses a Workspace mode
  without an image (NotFound).
- The window server (`door-window`) still embeds its own shell icons; moving it
  to the pack is a separate change.

## Alternatives rejected

- *A manifest per application.* It would change identity, policy and every
  consumer listed above for no isolation gain; IDs 16+ stay free.
- *Per-process copies (the desktop-wallpaper pattern).* That keeps one copy per
  process, the cost this change removes.
- *Reading assets through the storage service.* Allocation per process, no
  sharing, and a boot dependency on Storage for painting.
- *A checked-in generated pack.* It would duplicate 12.8 MB of repository bytes
  and allow the pack and the manifest to drift.

Contracts: `specs/contracts/native-window-v1.md` (Shared application resources),
`specs/contracts/initfs-v1.md` (generated resource packs).

## Addendum: fractional display scale in the application runtime (W9 part 3)

Applications connect with `CAP_DISPLAY_SCALE`; the HELLO DPI (96 x percent /
100) selects a `door_graphics_core::Scale` (100..300%) carried by `ui::View` as
`zoom`. Every logical edge maps to physical pixels with `Scale::px` (snapped
edges: abutting rectangles never seam, a 1 px line is one or two whole physical
rows), sizes with `Scale::len`, and the logical client size is
`Scale::logical_len`. `View::scale` stays the native asset density
(`asset_factor`): at 125/150/175% masks and bitmaps are area-filtered from the
2x frames to their physical size once and cached; content drawn at physical
resolution (Task Manager graphs) uses `View::physical_span`. Text uses the
125/150/175% hinted atlases from the pack (`FontAtlas::from_packed_percent`).
`SCALE_CHANGED` reloads the atlases before the CONFIGURED events that keep each
window's logical size. Host tests paint every application at 150% and reject
any half-alpha line pair (`raster_check`).

## Addendum: on-demand fonts and Unicode text

The full Windows fonts (Segoe UI Emoji, Historic, YaHei, JhengHei, Yu Gothic,
Malgun, Nirmala: 88.3 MB) stay whole and live in the authenticated initfs as
`fonts/*`, so they share the boot archive's identity check and need no new
boot-protocol record, ESP file or image role. `door_initfs::MAX_ARCHIVE` rises
from 64 to 192 MiB (raised to 256 MiB on 2026-10-24 when the IME dictionaries,
33.5 MB, were staged; that is the UEFI loader's existing largest allocation, and
the desktop archive is now about 200 MB). At the split it was about 167 MB: the pack is 20.3
MB (Stream T's Segoe UI faces), the GUI executables 31.7 MB (the Unicode engine
added about 2 MB to each application and the window server is 13.3 MB), so it
had outgrown 64 MiB even without the fonts.

Cost, measured on the host: software SHA-256 (the loader's and kernel's
`sha2` without CPU extensions) hashes 88 MiB in 210 ms (about 420 MB/s). The
loader and the kernel each hash the whole archive once at boot, so the fonts
add about 0.4 s of native hashing; under QEMU TCG (no KVM) expect several
seconds more boot time. Guest boot time is not yet measured. If it is
unacceptable, the follow-up is a separate font archive loaded beside initfs
with per-file SHA-256 from an authenticated manifest checked lazily on first
request, which needs a new optional BootInfo record and image role.

`operation::RESOURCE_OPEN(id)` names a font only by its fixed id
(`door_abi::platform::resource`); the kernel serves Window, Workspace, Preview
and Monitor processes, copies the file into one pinned section on the first
request (`MAX_RESOURCES` 8: the pack plus seven fonts) and grants
QUERY|READ|MAP. Processes map views read-only in `0x8000_0000..0x9000_0000`
(`door_rt::resources`), once per font for the process lifetime. Every process
builds one `TextSystem` from the pack's Segoe UI Variable, Segoe UI, Segoe UI
Symbol and Cascadia Mono faces with a `FontProvider` over those sections, and
attaches it to each atlas (monospace with `Family::Monospace`); the window
server does the same with the pack now granted to it in slot 7. ASCII keeps
the hinted atlas pixels; any other script, symbol or emoji is shaped and
drawn by the engine, so the display paths no longer replace text with '?'.
