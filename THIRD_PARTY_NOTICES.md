# Third-party dependency notices

Microsoft-owned resources (Windows fonts, icons, cursors, sounds and
wallpapers) named below are not distributed with this repository: `cargo
host-assets` copies them from the user’s own Windows installation
(`assets/host-assets.json`). Their mentions record where they came from.

The offline wallpaper collection additionally retains original Windows11 dark
Bloom, Glow and Sunrise JPEGs from the pinned windows-ui-assets commit
21970f48b8f66b4d44742cb30388c1454512d681. Each source and lossless decoded PNG
is identified in assets/wallpapers/windows/collection.json. Build-time QOI packing
and small bicubic previews use local sources only; no guest network dependency.

P11 adds ab_glyph0.2.32 (Apache-2.0), ab_glyph_rasterizer0.1.10
(Apache-2.0), owned_ttf_parser0.25.1 (Apache-2.0), ttf-parser0.25.1
(MIT OR Apache-2.0), core_maths0.1.1 (MIT), and libm0.2.16 (MIT).
Default ab_glyph features are disabled; the explicit libm/no-std-float path
supports original bounded guest text drawing. Third-party internals are not
claimed unsafe-free or exhaustively audited. Cargo.lock pins registry checksums.
Unmodified Microsoft Selawik1.01 regular/semibold fonts are under OFL-1.1;
copyright, reserved name and full license are retained in assets/fonts/selawik.
That directory's provenance.json pins archive, release commit and file hashes.
Selawik is provisional typography, not identical Segoe UI Variable.

The P10 ordinary image includes Ed Davis's Tiny Basic interpreter from
`https://github.com/eddavis2/Tiny-Basic`, commit
`fe4c942711c63cc932574b2f37a9ee628b001e31`, `C/int-stc.c`, unchanged.
Its Unlicense/public-domain text is preserved at
`third_party/apps/tinybasic/LICENSE`; exact input hashes are in `UPSTREAM.json`.
Original Door platform glue and explicit port limits are in `tests/windows-fixtures/tinybasic`.

P00 adopts supporting libraries only for host tooling. `Cargo.lock` preserves exact
versions and registry checksums; `specs/sources/dependencies.json` records features,
transitive edges, build-script hashes and the reviewed scope. The portable boot
protocol/image/initfs parsers have no dependencies. Those portable crates and host
tooling forbid unsafe; kernel memory and hardware modules isolate explicit unsafe
operations with separate host-test models. Third-party
internals contain unsafe optimizations and are not claimed to share that guarantee.

| Active host package | Version | Declared license |
|---|---|---|
| block-buffer | 0.10.4 | MIT OR Apache-2.0 |
| cfg-if | 1.0.5 | MIT OR Apache-2.0 |
| cpufeatures | 0.2.17 | MIT OR Apache-2.0 |
| crypto-common | 0.1.7 | MIT OR Apache-2.0 |
| digest | 0.10.7 | MIT OR Apache-2.0 |
| generic-array | 0.14.7 | MIT |
| itoa | 1.0.18 | MIT OR Apache-2.0 |
| memchr | 2.8.3 | Unlicense OR MIT |
| serde_core | 1.0.229 | MIT OR Apache-2.0 |
| serde_json | 1.0.151 | MIT OR Apache-2.0 |
| sha2 | 0.10.9 | MIT OR Apache-2.0 |
| typenum | 1.20.1 | MIT OR Apache-2.0 |
| version_check | 0.9.5 | MIT/Apache-2.0 |
| zmij | 1.0.23 | MIT |

Upstream repository locators are retained in the dependency record. Copyright and
license texts remain in each acquired registry package; a redistributed package
must preserve the applicable upstream texts and notices. This inventory does not
select a license for Door OS. No third-party source was copied into project code.

The reviewed build scripts perform compiler/configuration discovery and generated
module output under Cargo OUT_DIR; no network downloads or installers were
observed in those scripts. No proc macros are active for the tested host target.
Inactive/platform-specific lock entries are retained by Cargo and remain visible
in Cargo.lock. Guest adoption, release supply-chain audit and redistributed notice
packaging require their own gates.

QEMU is an owner-installed development tool. Downloaded EDK2/OVMF firmware from
QEMU v10.2.0 is used only in local emulation; source and exact binary hashes live in
`specs/sources/emulator-acquisition.json`. The acquired `edk2-licenses.txt` preserves
its full upstream BSD/patent and other component notices under the ignored acquisition
directory. Firmware implementation code is not part of Door's source tree.


P02 adopts pinned `uefi` 0.41.0 (MIT OR Apache-2.0), without its allocator/logger/
panic features, for supporting firmware ABI/protocol interfaces and final map/exit.
SHA2 0.10.9 is used allocation-free with force-soft for boot content identity.
The expanded 31-package registry inventory, licenses, manifest/build-script hashes,
proc-macro roles and scope are in the dependency review. Upstream notice text remains
in each acquired registry package; this does not select Door's project license.

P03 introduces no registry package or third-party source. Public Intel processor
manuals and UEFI Forum ACPI specification sources are consulted as hardware/format
references. Their retained acquisition identities are in `specs/sources/index.json`;
the documents are not included in a redistributed Door image. These reference
documents do not grant a redistribution license to Door's original source.

P06 normative references remain under ignored out/acquisitions/storage-reference.
They include OASIS, UEFI Forum, Microsoft and Linux documentation, plus interface
reference copies of QEMU GPL2-or-later sources and its Linux syscall-note register
header. No code from those references is linked or copied into the original Rust
driver/filesystem implementation. Reference identities are in specs/sources/index.json.

P07 uses Microsoft Learn's public naming, CreateFileW, access-check and registry
contracts as interface references. Exact retrieved HTML identities are recorded in
specs/sources/index.json and local copies remain in ignored acquisitions. Journal,
namespace and configuration code is original; no Microsoft implementation, binary
hive format or proprietary visual asset is incorporated.

P08 introduces only original workspace packages and no new registry dependencies
or third-party assets. The allocator contract reference comes from the installed
Rust1.94.1 documentation, retained under ignored acquisitions with its exact hash.
The existing Rust toolchain and dependency notices continue to apply.

P09 adds original workspace packages only. The installed Microsoft C compiler,
SDK headers and Kernel32 import library are invoked locally to produce the
independent fixture; they and Windows system DLLs are not redistributed. Original
guest DLLs use the pinned Rust compiler, prebuilt core/compiler-builtins and bundled
LLD under the existing toolchain notices. Public Microsoft interface references
remain ignored local snapshots with recorded identities; no implementation is copied.

P11 uses ab_glyph0.2.32, ab_glyph_rasterizer0.1.10, owned_ttf_parser0.25.1,
ttf-parser0.25.1 and libm0.2.16 in the bounded guest font path. Exact resolved
versions/licenses are authoritative in Cargo.lock and specs/sources/dependencies.json.
Selawik1.01 regular/semibold is OFL1.1; twenty-nine unmodified Fluent System Icon SVGs
at commit a347af95478f989038cccbf085978865264c048a are MIT. Full guest notices
are assets/licenses/desktop-notices.txt, installed as /system/share/desktop-notices.txt.
Host-only resvg0.45.1 and its dependency tree export alpha masks; no SVG renderer
is linked into the guest. Its package licenses remain in the pinned registry
sources and the complete dependency review. No extracted Windows shell implementation
is included. The local Windows reference resources are recorded below.

## Owner-requested Windows icon references

Eleven unmodified ICO files from HaydenReeve/WindowsIcons commit9442e3ef014520d4337409a24be10b970a36d66b are used in the local development image at the owner's explicit request. The repository declares no license. Origin and hashes are in assets/icons/windows/manifest.json; distribution rights have not been reviewed. ICO/PNG decoding and20/40px conversion run only in host tooling; guest pixels are bounded premultiplied BGRA.

Six Windows11 CUR files from bartekl1/windows-ui-assets commit21970f48b8f66b4d44742cb30388c1454512d681 are used for the owner-requested local development GUI. Acquisition hashes and source URLs are recorded under assets/cursors/windows. Distribution rights remain unreviewed.

The same pinned repository supplies the clean Bloom wallpaper; JPEG/PNG identities
and the1024x768 cover export are in assets/wallpapers/windows/manifest.json.
Unmodified Segoe UI regular/semibold from the owner's Windows installation are
identified in assets/fonts/segoe-ui/provenance.json. These resources are used in
the requested local reference image; no new redistribution rights are asserted.
# Current P12 codec and branding

qoi0.4.1 (MIT OR Apache-2.0), no_std+alloc, decodes the fixed lossless wallpaper
through safe Rust boundaries. Existing bytemuck1.25.2 is also used through this
codec. Door SVG app branding is authored in this repository and
exported with the existing host-only resvg pipeline. Original Windows references
remain recorded separately below.


P12 Settings categories also retain10 original Microsoft Support PNG files and
Microsoft Segoe Fluent Icons from the official design-resource download. Font EULA
and source/hash provenance are stored locally beside the font. E702/E895 glyphs
are rasterized offline; no font or asset is fetched by the guest.

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

## Installed known-folder artwork follow-up

Five additional known-folder role assets use the same installed Windows resource
provenance as Documents: Desktop183, Downloads184, Music108, Pictures113, Videos189
from imageres.dll.mun. Source records are assets/icons/windows/*-current.source.json;
see PROVENANCE.md and docs/design/native-viewer-title-gestures.md. Their acquisition
does not grant a new project license or establish redistribution rights.


## Explorer Windows reference artwork and preview decoding

Additional original Microsoft file-type and live folder icons are preserved from
HaydenReeve/WindowsIcons at commit9442e3ef014520d4337409a24be10b970a36d66b.
Their source/hash inventory is assets/icons/windows/manifest.json; the repository
is an artwork archive, not a license grant from this project. The PDF icon is the
installed Microsoft Edge PDF association resource. Segoe Fluent glyphs retain
Microsoft copyright and use the existing acquired-font provenance. No project
license or broad redistribution right is asserted. Owner screenshot references
are retained separately and never painted as the application itself.

zune-jpeg0.5.15, zune-png0.5.2, zune-core0.5.3 and zune-inflate0.2.54
are unmodified pinned Rust packages, licensed MIT OR Apache-2.0 OR Zlib as stated
by their package manifests. This use selects the MIT option; exact notices and
license texts and available source notices are retained in third_party/licenses/explorer-decoders. The older zune-inflate0.2.54 archive omits standalone license files; its published source notice and this limitation are recorded there. Package
checksums and feature/scope review are in specs/sources/dependencies.json.

## Door Text Unicode stack (P12, 23 October 2026)

door-graphics-text adds guest no_std packages: harfrust0.14.0 (MIT; HarfBuzz port,
HarfBuzz itself Old MIT), skrifa0.48.0, read-fonts0.45.1 and font-types0.12.7
(MIT OR Apache-2.0), unicode-bidi0.3.18, unicode-script0.5.8 and
unicode-segmentation1.13.3 (MIT OR Apache-2.0), unicode-linebreak0.1.5
(Apache-2.0), spin0.12.3 and core_maths0.1.1 (MIT), yoke0.8.3 and zerofrom0.1.8
(Unicode-3.0), plus existing bitflags, bytemuck, smallvec, once_cell,
stable_deref_trait and ab_glyph_rasterizer. Exact versions, features and checksums
are in Cargo.lock and specs/sources/dependencies.json; licence texts are in the
pinned registry sources. A distributed binary must carry these notices (the guest
notice file assets/licenses/desktop-notices.txt is maintained by its owner).

Owner-installed Windows fonts (Segoe UI Emoji, Segoe UI Symbol, Segoe UI Historic,
Microsoft YaHei UI, Microsoft JhengHei UI, Yu Gothic UI, Malgun Gothic, Nirmala UI,
and the existing Segoe UI / Segoe UI Variable copies) are owner-local Windows
fonts, not redistributable, private build. They are recorded in PROVENANCE.md and
assets/fonts/windows-unicode/provenance.json and must not be published.

IME dictionaries (when acquired with `cargo xtask ime-dictionaries`; see PROVENANCE.md
and assets/ime/sources.json): CC-CEDICT © MDBG and contributors, CC BY-SA 4.0
(attribution and share-alike apply to assets/ime/pinyin.dict); jieba word frequencies
© Sun Junyi, MIT; Mozc dictionary data © Google LLC, BSD-3-Clause, including data
derived from IPAdic (© Information-technology Promotion Agency, Japan, IPA licence).
A distributed image carrying these files must include these notices and licence texts.

## P14 credential hashing (26 October 2026)

The session service (door-system-services) links the guest no_std packages
argon2 0.5.3, blake2 0.10.6 and base64ct 1.8.3 (RustCrypto, MIT OR Apache-2.0) and
subtle 2.6.1 (dalek-cryptography, BSD-3-Clause), with
the existing digest and cpufeatures packages. Licence texts are in the pinned
registry sources; a distributed binary must carry these notices.
