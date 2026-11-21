# Multiwindow/Notes glyph extension — 22 October 2026

Three additional Segoe Fluent selectors (Bold e8dd, Italic e8db, Strikethrough ede0)
use the existing pinned font and native20/40/60 offline export pipeline. System
reuses the existing display glyph. No font binary or third-party dependency was
added. Mapping: https://learn.microsoft.com/en-us/windows/apps/design/iconography/segoe-fluent-icons-font

# Source and asset provenance

The22October Monitor/Preview revision uses the owner concepts recorded in
reports/runs/reference-apps-owner-20261022/index.json. They are design references,
not implemented backends or actual Windows captures. Forty-six app-only Segoe
Fluent glyph selectors use the already pinned local font and Microsoft glyph
index; masks are offline native20/40/60 exports in the existing manifest. No new
font binary or dependency was acquired. Native icon provenance/reuse limits
remain as recorded below.

The 3D navigation glyph additionally uses Microsoft's MIT-licensed Fluent Cube
SVG at pinned commit a347af95478f989038cccbf085978865264c048a. Its original source,
SHA-256 and A8 exports are in assets/icons/fluent/manifest.json. It is an app-only
asset; the existing Workspace icon enum and binary budget are unchanged. This is
an acquired design asset, not Windows implementation source.

P10 ordinary-session application adoption: the pinned Ed Davis Tiny Basic source
and license are preserved in `third_party/apps/tinybasic/UPSTREAM.json`. The C
interpreter is unchanged; `tests/windows-fixtures/tinybasic` supplies original narrow platform code.
MSVC/SDK compile it against public Win32 imports, and the same PE is compared on
Windows and Door. This source-port trial is distinct from stock binary support.

The supplied master specification is preserved byte-for-byte. Its original locator
and SHA-256 are recorded in `project/baseline.json` and `specs/sources/index.json`.
It supplies project requirements; it does not prove an implementation or result.

Project-owned P00 Rust code was authored for this workspace with AI assistance from
the supplied original BootInfo design, public Rust documentation and the UEFI
memory descriptor contract. No Windows, Wine or ReactOS implementation source was
used. P09 subsequently acquired local Windows reference observations as recorded
below. At that original P00 checkpoint no Microsoft design assets had been acquired.
Subsequent reference assets are recorded here and in the pinned manifests.
This is a provenance workflow; it does not certify training history or establish
strictly separated clean-room teams.

P09 introduces original PE dependency/fixup planning, compatibility ABI views and
ntdll/kernel32 sources. Microsoft Learn and the installed SDK10.0.19041.0 supply
public format/layout/API facts; exact reference/header hashes are in the source
ledger. The ordinary original C fixture uses owner-installed MSVC/SDK tools and
the SDK import library. Its actual include trace hashes all used headers. The same
EXE bytes are retained for local Windows10Pro19045 reference and Door comparison.
No Microsoft system DLL or implementation source enters the guest image. The
original DLL compiler runtime has an explicitly aborting exception entry, which
does not claim Microsoft C++ runtime or exception compatibility.

`door-boot-protocol` and `door-image-format` have no third-party dependencies.
P01's original inspectors use public ELF/PE contracts; QEMU documentation supplies
supporting-tool argument and disk contracts. Shared format fixtures are original
synthetic bytes and are never runnable OS/Windows evidence. `door-xtask` uses JSON and
SHA-256 support from the reviewed, locked host dependency graph. See
`specs/sources/dependencies.json` and `THIRD_PARTY_NOTICES.md`. These libraries
execute on the host and are not guest OS components.

P02 loader, paging, handoff, kernel diagnostics and constrained archive logic are
original project code. Supporting `uefi`/SHA2 libraries are explicitly adopted and
pinned with audited feature/lifetime boundaries. UEFI Forum's public release sources
were acquired for boot services, media and GOP; no foreign kernel/compatibility code
was copied. Diagnostic glyphs and door mark are original. Real guest screenshots
come from QEMU's framebuffer, with no hosted visual substitution.

P03's physical-frame allocator, early heap, page-table ownership, descriptor/trap
assembly, APIC timer and portable ACPI readers are original project code. Intel's
public system-programming manual 325384-081US and UEFI Forum's ACPI 6.5 release
sources were acquired and inspected for hardware/table contracts; exact source
URLs, byte counts, hashes and consulted sections are in `specs/sources/index.json`.
Acquired references remain under ignored `out/acquisitions`; they are reference
documents, not adopted operating-system implementations. P03 adds no third-party
Rust dependency. The pinned UEFI wrapper also supplies configuration-table and
memory-map access before exit. Ownership and parsing remain in Door code.

Add a source record before using a new normative contract. Distinguish an acquired
public document, an observed behavior and an unknown. Record versions, locator,
review, supported facts and unresolved questions. Exclude leaked/confidential code
and copied compatibility implementations. A strict clean-room claim would require
a separate access policy and reviewed specification handoffs.

The project owner has not selected a redistribution license. The specification,
new source, third-party libraries, firmware, fonts and future fixtures have distinct
rights. Current local work does not authorize publication, trademark claims or
redistribution of future third-party assets. OVMF is now downloaded for local
emulation under ignored `out/acquisitions`, with complete upstream notices and a
tracked acquisition record. No firmware source or visual assets are bundled into Door.

P05's private page tables, user entry/copy assembly, object authority, process
runtime and IPC are original Door implementation. Native ABI values and layouts
are project-owned contracts; the separate raw assembly fixture intentionally
repeats those published literals. Intel's already acquired Volume 3 also supplies
the SYSCALL/SWAPGS/IRET contract (including section 5.8.8). No new registry packages,
foreign OS code, host syscall adapter or third-party runtime were adopted. The two
native ELF fixtures are generated from tracked original source and packaged by
content identity into initfs.


P06 adds original Rust PCI/VirtIO transport, block/VFS/filesystem code and host
GPT/FAT32/ext2 fixture formatting/auditing. OASIS VirtIO 1.2 CS01, UEFI 2.10 GPT,
Microsoft FAT 1.03 and Linux-published ext disk-layout documentation supply the
interface contracts. Pinned QEMU PCI sources were consulted for register facts,
not incorporated. Exact acquired identities and failed direct-download fallbacks
are in the source ledger. No new registry dependency or filesystem port is adopted.

P07 uses Microsoft Learn's public naming, CreateFileW, access-check and registry
contracts as interface references. Exact retrieved HTML identities are recorded in
specs/sources/index.json and local copies remain in ignored acquisitions. Journal,
namespace and configuration code is original; no Microsoft implementation, binary
hive format or proprietary visual asset is incorporated.

P08 adds original native platform ABI, process/thread/session authority, runtime
allocator/TLS adapters, four protected services, their versioned RPC SDK and an
independent native application. Storage/persistence code is moved into the shared
`door-storage-core` crate rather than copied. No third-party guest dependency is
introduced. Rust1.94.1's installed GlobalAlloc documentation supplies allocator
ownership and failure contracts; the exact acquired copy is in the source ledger.
Development logon capabilities are project-owned authority objects, not a claimed
password verifier or entropy implementation. The text console uses the existing
original glyphs and native framebuffer output.

P10 adds original allocation-free PE lifecycle/TLS/unwind validation, native-backed
Windows base adapters and an original scoped Msvcrt implementation. All Windows
reference fixtures use public SDK/C ABI contracts, pinned installed compiler,
linker, MASM and headers. The C TLS support is original fixture runtime code;
Msvcrt memory/string/heap code is original Rust. Windows reference processes use
the host OS's known DLLs, while Door initfs contains only the original project
DLLs. No Windows DLL implementation, ReactOS/Wine source or host CRT binary is
copied into the OS. No third-party registry dependency or feature changed.

P10 final qualification is bound in project/phases.json to report
host-1792214999674635300-12500 and reports/critics/p10-critic.json. The independent critic
reviewed 285 exact build inputs and the same original fixture artifacts in both
environments. The installed SDK/compiler contracts and unchanged dependency
review remain in specs/sources; no additional implementation source, guest
binary, registry dependency or proprietary visual asset was adopted.

## P12 cached shadows and installed Documents reference

The finite Gaussian shadow coverage and its host generator are original Door
work; no Windows compositor implementation or shadow texture was copied. The
14,723-byte cache is not claimed identical to DWM. Documents known-folder artwork
was read from the owner's installed Windows imageres.dll.mun group112 without
loading the module; exact module/icon hashes and eight authored frames are pinned
in assets/icons/windows/documents-current.source.json. Folder/sidebar/caption
roles use it; the separate Notepad text-file association is unchanged. This is
an owner-provided local development reference, not a new redistribution grant.
No package/version/feature/unsafe change accompanies this checkpoint. Details and
bounded evidence are in docs/design/cached-window-shadows.md and
reports/critics/p12-cached-move-critic.json.

## Current known-folder roles

Desktop183, Downloads184, Music108, Pictures113 and Videos189 were acquired by
the checked PE-byte importer from the installed imageres.dll.mun referenced by
desktop.ini. The module SHA256 is
d816fd64e8c1c26014778c9667234f7459814bd38aa04e35c46a25073d0876ee.
Each assets/icons/windows/*-current.source.json pins its ICO and eight authored
frames. Independent review matched40 resource payloads and35 small BGRA exports.
Documents112 retains its earlier accepted source. These are known-folder roles,
not a new association for every file type; Downloads navigation keeps its arrow.
See reports/critics/p12-native-viewer-title-critic.json.

Terminal native preview (22 October 2026) uses original Rust application code, the existing native Window server and pinned Windows glyph/icon/font assets. No new third-party package or foreign shell source was added. The existing door-userland native parser is reused; it does not qualify CMD or PowerShell. User image identity is retained at reports/runs/user-reference-20261022-terminal/index.json. See docs/design/terminal-native-preview.md.

Terminal icon refinement uses WindowsIcons terminal.ico, settings.ico and info.ico from the same pinned commit as the existing icon manifest. Exact file SHA-256 values are in assets/icons/windows/manifest.json. Owner-installed Cascadia Mono (C:/Windows/Fonts/CascadiaMono.ttf) is pinned in assets/fonts/cascadia-mono/provenance.json for local terminal text; GUI text retains Segoe UI. No new third-party code package was added.

Terminal reference correction: the client uses the existing trusted Window server frame, pinned Segoe UI/Cascadia Mono and Segoe Fluent glyphs. The owner approved proper official profile icons from Microsoft Terminal/PowerShell and Canonical Ubuntu WSL on22October2026, superseding the earlier WindowsIcons-only preference for missing profile artwork. Source commits, exact URLs, lossless PNG-to-ICO wrapping and SHA256 identities are recorded in assets/icons/windows/manifest.json. No engine binaries were downloaded or executed. Local-development asset provenance is not a redistribution-rights determination.


## Explorer reference revision, 22 October 2026

The owner supplied the exact Explorer screenshots retained in
`docs/references/explorer-20261022/sources.json`; images are reference evidence,
never screenshot-backed application content. Native geometry/painter, tab and
inspection/tag controllers, bounds checks and tests are original project code.
No Windows Explorer implementation source was copied. Color file icons and live
folder layers were downloaded from HaydenReeve/WindowsIcons at pinned commit
9442e3ef014520d4337409a24be10b970a36d66b; exact URLs, SHA256 and ICO bytes are in
`assets/icons/windows/manifest.json`. The PDF association artwork was read from
the installed Edge named PE resource, without loading module code; provenance
records its installed version/group/module hash. Commands use the pinned local
Segoe Fluent font with exact codepoints in the Windows-shell manifest; blue and
neutral tint roles retain that glyph geometry. The rejected invented file atlas
and its exporter were removed. Small native exports and lossless QOI hero assets
are generated offline by xtask and embedded; no guest network or SVG parser.

PNG/JPEG decoding adds pinned zune-jpeg0.5.15, zune-png0.5.2,
zune-core0.5.3 and zune-inflate0.2.54. Registry checksums remain in Cargo.lock;
current metadata/licenses/features and the bounded call-site review are recorded
in specs/sources/dependencies.json. No dependency-internal exhaustive audit or
sandboxed decoder-worker claim is made. Small generated transparent-PNG/JPEG
color fixtures test actual decoding and premultiplied alpha; sample names and
sizes in host painters are explicit fixtures, never real guest content. Guest
persistence testing uses a newly formatted private raw disk, not owner data.

Preview and Monitor expansion (22 October2026): original vector icons and native
application/telemetry/PNG-export code. Branding identities are recorded in
assets/branding/preview-monitor.source.json. The 26 October 2026 redraws (Preview:
a flat, full-bleed blue-to-violet gradient square with no inner frame, base or
border, a white sun and two soft ridges, the cyan front ridge rising on the Door
open-door diagonal; Monitor/Task Manager: pulse line on a blue tile) are original
Door SVG, not traced from any Microsoft icon, exported by the same offline resvg
0.45.1 branding pipeline (`cargo xtask assets branding build` reproduces every
Preview byte). Owner concept images, when their
original files remain available, are preserved under assets/references/owner-apps-20261022
with exact hashes and visual-reference-only scope. Windows file/control icons
retain their earlier acquisition manifests; small QOI repacking is lossless.
Generated1MP RGBA/16-bit PNG and JPEG fixtures are original deterministic Pillow
patterns, not owner photographs. No new external package version is added;
door-app-ui, door-preview and door-monitor are local workspace packages.

## Door Text Unicode fonts and shaping (23 October 2026, workstream T)

Owner decision 2026-10-23 (docs/design/p12-p13-plan-20261023.md): this private
build uses the owner's installed Windows fonts. Read-only byte copies from
C:/Windows/Fonts, compared byte-identical after copying; host files untouched.
Every file is an owner-local Windows font, not redistributable, private build.
Fonts are loaded at run time from caller-supplied bytes, never `include_bytes!`.

| File | Bytes | SHA-256 | Role |
|---|---|---|---|
| assets/fonts/windows-unicode/seguiemj.ttf | 12934920 | 451df9bb5546055af625ff86aa72695df43e8e170b4c6c1316fe44e9c7a22f7c | Emoji (COLR/CPAL) |
| assets/fonts/windows-unicode/seguisym.ttf | 2514056 | a4a35dcc62cd30e1a6c97b695ecef83e59d2b149e3af834f6a49f11851d56b37 | Symbol |
| assets/fonts/windows-unicode/seguihis.ttf | 1622060 | a162ceeaf701b55d11c542daf1c39c52bfaf686496505bff2eb75ca24814c136 | Historic |
| assets/fonts/windows-unicode/msyh.ttc | 19704352 | d79c55e68b1131eea0cc1c47be4f572d964f28c682e143db2ad09c1e4cb07a3f | Microsoft YaHei UI |
| assets/fonts/windows-unicode/msjh.ttc | 21404172 | 1fd5fc7046e0eea753988ee06d1a5a731df68475614e2174fbba9b4800f392ac | Microsoft JhengHei UI |
| assets/fonts/windows-unicode/YuGothM.ttc | 13908392 | cb2021bbfe40b21ec564848b8d702125661594e1b4f1eb2e77fd8a98b485cf19 | Yu Gothic UI |
| assets/fonts/windows-unicode/malgun.ttf | 13459196 | 7a183cf1c6c56b9609fcc16eda8b5229fbc11758a21e669ec00343239b02192f | Malgun Gothic |
| assets/fonts/windows-unicode/Nirmala.ttc | 5304988 | ad02cdfc06e144ac45f318e8e5a64cbe04c7479d4beb91d25f5a319a466b1767 | Nirmala UI (Hindi) |

The existing pinned Segoe UI Variable (27ca1ab4…), Segoe UI (ba32a222…) and Segoe
UI Semibold (98532834…) in assets/fonts/segoe-ui are reused for the UI roles.
Per-face selection and notes are in assets/fonts/windows-unicode/provenance.json.
Shaping, segmentation, bidi, fallback and rasterization are original Door Text code
over pinned open-source crates (specs/sources/dependencies.json,
`p12_door_text_20261023`). This is not Microsoft DirectWrite or ClearType. The
multilingual corpus tests/fixtures/text/corpus.txt is original text written for
this project; its golden renders are generated from the fonts above.

## IME dictionaries and keyboard layouts (P12 workstream I, 23 October 2026)

Keyboard layout tables (`frameworks/keyboard-layouts`) are original code following the
documented Windows layouts (KLIDs in `specs/contracts/text-input-v1.md`). The IME
engines (`desktop/ime`) are original code; Korean composition is algorithmic. No
dictionary data is stored in source. `cargo xtask ime-dictionaries --fetch --pin`
downloads these upstream files into `assets/ime/upstream/` (owner approval required;
not yet acquired) and converts them to `assets/ime/pinyin.dict` and
`assets/ime/japanese.dict`. SHA-256 pins are recorded in `assets/ime/sources.json`
at acquisition.

| Source | URL | Version | Licence |
|---|---|---|---|
| CC-CEDICT (MDBG) | https://www.mdbg.net/chinese/export/cedict/cedict_1_0_ts_utf-8_mdbg.zip | rolling export, pinned by SHA-256 | CC BY-SA 4.0 (derived pinyin.dict is CC BY-SA 4.0) |
| jieba dict.txt (word frequencies) | https://raw.githubusercontent.com/fxsjy/jieba/v0.42.1/jieba/dict.txt | v0.42.1 | MIT |
| Mozc dictionary_oss (dictionary00–09.txt, id.def) | https://raw.githubusercontent.com/google/mozc/master/src/data/dictionary_oss/ | master at acquisition, pinned by SHA-256 | BSD-3-Clause; IPAdic-derived entries carry the IPA licence notice (verify the README at the pinned revision) |

## Time zones, regional formats and collation (P12 localization, 24 October 2026)

`frameworks/locale` is original code. `src/tz.rs` embeds 49 zones whose standard
offsets and recurring daylight-saving rules (US/Canada, EU, Australia, New Zealand,
Egypt) were transcribed by hand from the IANA Time Zone Database, release 2025b
(https://www.iana.org/time-zones, public domain). Only the rule in force today is
kept; historical transitions are not. The Windows display names in that table and
the regional format patterns in `src/region.rs` (en-US, en-GB, de-DE, fr-FR, zh-CN,
ja-JP, ko-KR) are factual Windows 11 defaults written from observation; no Microsoft
code, NLS data files or ICU/CLDR data files are copied. The collation in
`src/collate.rs` is an original StrCmpLogicalW-style algorithm. Record:
`specs/sources/dependencies.json` key `p12_localization_time_20261024`.

## P14 credential hashing, entropy and fidelity records (26 October 2026, WS-C)

Owner decision 2026-10-26 (ADR 0020). The session service hashes the sign-in
password or PIN with Argon2id from the pinned RustCrypto crates argon2 0.5.3,
blake2 0.10.6 and base64ct 1.8.3 (MIT OR Apache-2.0) and subtle 2.6.1
(BSD-3-Clause), acquired from
crates.io with exact locked resolution (checksums in Cargo.lock; review record
`p14_credentials_20261026` in specs/sources/dependencies.json). No upstream source
is copied into project code. The kernel entropy source (RDSEED/RDRAND, TSC jitter,
SHA-256 conditioning) is original code over the existing pinned sha2 0.10.9.

The reference profile profiles/windows-reference/reference-26h2-x64.json, the
design tokens in frameworks/design-tokens and the fidelity thresholds are original
measurements and code derived from the owner's own Windows 11 Pro 26H2 (OS build
26300.9711) captures in reports/artifacts/reference-26h2 (owner-approved observation of
apps and flyouts, no setting changes). No Microsoft code or binary is copied; the
captures stay local evidence. The door mark mask (assets/branding/door-mask.*) is
an original derivative of the original door mark.
