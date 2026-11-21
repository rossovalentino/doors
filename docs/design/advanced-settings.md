# Current audit validation - 22 October 2026

The current six-page common-image native check passed, including theme probes, an independent typed registry disk audit and Terminal launch from the four-app Installed apps page. Both applications retired cleanly. See current-app-fidelity-audit-20261022.md for current evidence and remaining provider limits.

# Generated evidence cleanup — owner action, 22 October 2026

The owner deleted the large generated report/test-data directories after they
exceeded100GB. Source and assets remain. Earlier report and screenshot paths
below may now be unavailable; they must not substitute for fresh validation.
New Settings output is limited to a focused private guest and compact reference
captures, sharing one current image with the other GUI chats. P12 remains partial.
# Advanced Settings visual refinement — 22 October 2026

The six-page owner references are being compared at their1472x952 client extent.
The current revision adds hinted16px regular/semibold typography, finite cached
Gaussian card shadows, Windows registry/service/computer icons, aligned table
columns, lighter list actions and tab treatments, and reference-position registry
panes. Laptop, monitor and tablet screens use the pinned classic Bloom in the
current light/dark theme. All18 lossless theme/device/density variants pass pixel
checks. Original frames and pinned Windows screen artwork have separate provenance.

The actual painter has twelve host review captures (six pages, two themes), with
explicit review fixtures excluded from guest builds. Latest native diagnostic
report host-1792603189607660900-11340 passed18 checks including six actual pages
and an independent durable registry audit, but failed source stability. It does
not qualify later refinements. The latest cumulative verify report
host-1792604089881300400-6712 stops on the dependency-review lock mismatch. Full
Windows providers, exact1:1 fidelity and smooth frame cadence remain unqualified;
P12 is partial and the owner volume is preserved. See docs/design/advanced-settings.md.
# Six Advanced Settings pages — owner revision, 22 October 2026

The current owner references supersede the earlier sixteen-item Advanced group.
Advanced now contains only Devices & drivers, Disk management, Services, Registry,
Group policy and Task scheduler. Other tools retain stable IDs and live under
System, Network, Accounts, Privacy or Update. The six retained reference images
and their input hashes are in docs/references/advanced-settings-20261022.

Native layouts follow the reference sidebar, breadcrumbs, headers, cards,
master/detail tables and three-pane editors. Compact windows stack panels;
registry panel heights grow with bounded real rows. Search, keyboard focus,
registry typed edit, session favorites/recent and selection use shared native hit
geometry. No screenshot sample hardware, Windows tasks or policy states are seeded.
The GUI is reference-inspired and still requires visual comparison; pixel-exact
identity and smooth frame cadence have not been established.

Device artwork is original SVG with1x/2x/3x native exports. The validated boot
handoff carries the OEM ACPI profile to a public read-only inventory syscall:
profile2 selects a laptop,8 a tablet, and other/unknown profiles a monitor.
Unspecified type stays explicitly unspecified. This is firmware advice and does
not infer convertible posture or physical chassis from the development host.

Storage now returns validated GPT extents and device writable/flush capabilities
through an authenticated read-only DiskInfo operation. The page shows the actual
mounted device, Ext2 user volume and transaction journal. Free-space accounting,
other devices and partition mutations are unavailable. Device writability is not
an authorization to repartition. Registry reads/checked typed writes and explicit
path creation remain durable. Services shows registered endpoints, not live health.
Group policy and Task scheduler lack providers and show explicit empty states;
Windows-equivalent enforcement, scheduled execution, driver update/install,
service control, partition editing, registry backup/restore/delete remain open.

Focused native validation command: cargo xtask test settings-reference-current.
This captures six actual pages on a private1920x1080 guest and independently audits
a registry value in raw disk bytes. It is a bounded workflow, not cumulative phase
qualification. The earlier287-check report had zero failed checks but failed its
source-stability condition; it is preserved and does not qualify these revisions.
P12 remains partial. The owner volume is never used as a test fixture.

Earlier implementation details follow as historical scope; sixteen destinations
remain internally implemented, but only the six above belong to Advanced.

# Advanced Settings implementation

The owner's 21 October 2026 reference supplies sixteen administration destinations.
All sixteen now have native surfaces inside Settings. The independently scrolling
rail has a collapsible Advanced group, title search, selection reveal, scrollbar
dragging, Back and Forward (Alt+Left/Right). Existing page IDs 0–17 are preserved;
additional pages append at 18–32. The existing Taskbar, Context menu and appearance
settings retain their durable service behavior. System rows link to the relevant
administrative destinations.

This is partial functionality, not Windows administration parity. Most missing OS
providers cannot be supplied by a settings control. Unavailable operations have no
hit target and no success state. The Settings pages explicitly identify those
provider requirements. No driver, disk, network, policy, event, task, account,
power or backup management implementation is claimed by this batch.

## Available behavior

* Registry: HKCU, HKLM and merged HKCR, separate 32/64-bit views, path entry,
  parent/child browsing, explicit create/open path, bounded coherent enumeration,
  new values and edits for all seven supported types. Existing names and UTF-16
  identities remain lossless. Values are limited to the backend's 64 bytes.
  Numeric overflow, malformed hex and bad escapes fail before mutation. Edits
  compare the observed key version in the serialized Storage owner; flush and
  close must succeed before the editor reports success. Failed edits retain their
  draft. Refresh clears stale actionable listings. Drafts survive category changes;
  navigating the registry itself requires Save or Cancel. Deletion, rename,
  import/export, ACL editing and arbitrary Windows registry hive support remain open.
  Each draft owns its captured location/version, independently of the visible
  address. Address encoding uses `%XXXX` for non-ASCII UTF-16 units (including
  unpaired surrogates) and `%0025` for a literal percent; untouched displayed
  addresses round-trip exactly. The name display's `\uXXXX` notation is separate.
* Services: authenticated Init endpoint discovery for Storage, Session, Console
  and Window. Discovered handles are immediately closed. Registration is labeled
  as a discovery snapshot, not process health or start/stop authority.
* Environment: exact inherited launch snapshot. Durable user/system environment
  edits and expansion/inheritance configuration remain open.
* Users & groups: kernel-supplied UID, PID, group bitmap, integrity, capabilities,
  session and epoch. Account creation and membership edits remain open.
* Disk management / Permissions & sharing: authorization-checked local object
  metadata lookup by editable native path, including identity, version, kind and
  file bytes. This is not capacity accounting, device inventory or an ACL editor.
* Diagnostics / Developer: explicit native ABI, monotonic clock and Storage read
  checks; links to installed native apps, registry and existing device information.
* System configuration: links to the existing persistent appearance, taskbar and
  context-menu controls; boot/service policies remain unavailable.

Service reads occur on entry or explicit Refresh/F5, never in painting. Provider
errors are visible. Mutations are never automatically replayed after connection
loss. The existing authenticated SDK, registry browser and typed-value models are
reused; those models were already present before this integration.

## Artwork and validation

`assets/icons/advanced/settings.svg` is original vector artwork with sixteen tiles
in the reference's order. `cargo xtask assets advanced build` exports consecutive
premultiplied BGRA tiles at 20, 32, 40, 60, 64 and 96 pixels. The normal read-only
`cargo xtask assets check` reproduces and compares every export. There are no new
dependencies or runtime SVG/network requirements.

Host tests cover stable navigation, collapsed search, independent rail clipping,
typed-value round trips and malformed input, coherent enumeration and checked
writes. Native guest validation extends the wide private-disk workflow with all
sixteen page captures, path creation, all seven value types, invalid edit
cancellation, immutable draft targets, pointer/search focus, pointer dismissal in
a scrolled short window, fresh Back/Forward transitions and an independent
journal/catalog audit. The owner volume is never
used as a test fixture. Evidence and independent critic results are recorded in
the current project checkpoint; P12 and cumulative qualification remain open.

## Reference refinement checkpoint, 22 October 2026

The source and painter now share adaptive Services toolbar geometry and Task
Scheduler card/Edit geometry. At612px width Refresh has its own row above Search;
at1129px collapsed task cards carry their Edit controls down with them. Regression
coverage exercises both themes, actual painter pixels/hits and five layout widths.
The earlier12host review captures remain a visual diagnostic, not a guest receipt.

The device hero uses original vector glass folds and original device frames with
pinned Windows classic Bloom screen pixels. Device icons and the sixteen internal
tool tiles are exported offline; tool tiles use deterministic lossless QOI to
retain the loader limit. Existing Windows registry, service-gear and computer
assets replace those three original category icons. The original six font styles
retain their sizes and indices; regular16 and semibold16 append two eager hinted
ASCII styles. No runtime SVG parsing, blur filtering or font rasterization occurs
while painting. These design choices do not establish a frame-rate guarantee.

A fresh final shared guest comparison is pending after the concurrent GUI chats'
source/document checkpoint. The cumulative gate and full Windows provider work
remain open. Additional Windows legacy administration candidates include Event
Viewer, certificate management, advanced firewall rules, Performance Monitor,
and Local Users and Groups; several already have merged destinations or require
services outside this GUI batch.