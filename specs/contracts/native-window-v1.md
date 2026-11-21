# P12 multiwindow extension — implementation under validation

Each native app class uses one authenticated connection and up to four independent
window models. CAP_MULTI_WINDOW128 negotiates trusted TAB_TARGET and catalog-only
LAUNCH_WINDOW; only negotiated batch pollers receive NEW_WINDOW13. Tab targets
require captured source authority, the topmost committed same-owner caption, and
coordinates outside protected system buttons. No foreign owner ID is disclosed.
A detached tab remains in its source until the destination commits its first frame;
timeout, cancelled admission or reconnect retires only the unadmitted blank window.

Current bounded capacities are12processes,64handles/process,256executive objects,
32channels and32sections; Window admits8owners/24windows/32buffers,4windows and
8buffers per owner,128MiB per owner and192MiB aggregate descriptors plus captures.
Producer mappings reserve disjoint4MiB-granule ranges; registration/resize failures
retain old model state. Workspace/Monitor heaps are96MiB; Preview is128MiB.
Telemetry v2 is864bytes/12records; the read-only monitor syscall still accepts the
legacy608byte/v1 request and returns at most8records. Reserved tails stay checked.
These are finite development limits, not a claim that every combination fits
physical memory or that all four windows can be maximized simultaneously at4K.

Native qualification is pending. Earlier contract/evidence below remains historical.

# Native window and CPU display contract (P11)

Current P12 rendering extension: fixed boot densities are 96/192/288 DPI
(1x/2x/3x); 4K defaults to 300%. DISPLAY_INFO.max_copy and DISPLAY_WRITE share
the 16 KiB bound in door_abi::graphics::MAX_COPY_BYTES. Capture validates the
complete caller range into private kernel bytes before device output. BGRX GOP
accepts BGRA bytes directly (the fourth byte is reserved); RGB output still
swizzles. Padded physical rows are preserved. This remains CPU copy without
vblank/page flip. Earlier 4096-byte and 96-DPI descriptions below are historical.
See docs/design/rendering-responsiveness.md for current scope and verification.

Status: adopted P11 contract; phase qualification is recorded in project/phases.json.
Windows visual-fidelity acceptance remains a separate P14 gate.
Requirements: master 41Ã¢â‚¬â€œ48, 91, 114, 140; WINDOW-A and WINDOW-B.

The protected `door-window` process contains separate protocol/window authority,
portable scene/composition, input-policy and display-transfer modules. The kernel
catalog grants manifest 11, UID 3, session zero, one thread and a256MiB heap the
sole Display capability. Ordinary clients have neither this capability nor a GOP
mapping. Manifest 12 is an unprivileged session process with a 4 MiB heap. Files
and Notes are independent processes using the public window and storage SDKs.
Original text session boot and generic ELF/PE admission retain their P10 contracts.

`door.window` uses the existing authenticated service envelope/version, bounded
little-endian records and kernel peer identity. Opcodes and record layouts live in
`crates/service-protocol/src/window.rs`; clients never serialize an owner. INIT
alone binds the active `(session, epoch, uid)`. An owner additionally includes its
kernel process incarnation. A service restart creates a new authenticated endpoint
incarnation. No old Create/Commit is replayed; clients explicitly recreate windows
and repaint from their retained application data. Old geometry is not restored yet.

Hello declares version, physical extent, 96-DPI development scale, premultiplied
BGRA8888/sRGB and CPU snapshot capability. It reports bounded resources.
Hello also reports live windows, buffers and the caller's pending server attachments for
lifecycle/accounting checks; the last two words remain reserved zero. Unsupported
versions, roles and operations fail explicitly. Configure events carry the current
outer geometry and exact content size. Acknowledgement and Attach must name that
epoch and an owned generational resource. Title strings are bounded printable ASCII
until P12 Unicode/shaping. RequestState supports minimize/maximize/restore; standard
caption controls, pointer drag/resize, Alt+Tab and Alt+F4 use trusted server policy.
Left/right/top/bottom border drags anchor the opposite edge. Focus changes revoke
old pointer capture; a late capture request returns an error without killing the
consumer. Clipboard, arbitrary transforms/effects, child/modal policy, fences/GPU imports,
secure shell roles and Windows GUI adapters remain separate phase requirements.

Buffer registration captures and validates width, height, padded stride, full
backing extent and a read-only Section grant. The kernel checks the real backing
range, independently of the declared descriptor. The compositor never maps that
section. Commit captures at most 65536 bytes per kernel call into owned pixels using
the bounded assembly copy/fault-fixup boundary under a live section lease and VM
guard. Complete output buffers are validated and must be private; shared-section
output aliases are denied. There is no ordinary immutable Rust
reference into concurrently writable producer memory. Concurrent malicious writes
can affect only that submitted image; invalid alpha/extent is rejected and the
last safe image survives. No zero-copy ownership or memory sealing is claimed.

Successful capture publishes a private immutable snapshot and enqueues exactly one
BufferReleased. That snapshot survives producer reuse/unregister. Attach alone
transfers no pixels. Configure/close/destroy cancel pending reservations; stale
IDs/epochs cannot address replacements. Input and release events have bounded
backpressure; superseded Configure and motion events coalesce. Advisory focus,
capture and presentation notifications can coalesce/drop with reconciliation APIs.
A saturated input consumer loses its window state rather than silently losing a
key-up. Disconnect destroys all owned windows/buffers/snapshots and capture.
Evicted channels return Closing until they disconnect; they cannot silently
re-register a fresh scene on the same channel. A responsive owner can CancelClose
after a failed save; that cancels the deadline and restores input/focus.
Queued pointer coordinates are content-local at receipt time, so moving a cached
window never leaves an application using an obsolete global content origin.

SetCursor (0x5015) accepts exactly a window ID (u64) and shape (u32): 0 arrow,
1 text. The authenticated window owner may change only its client-area pointer.
Unknown values, malformed sizes and foreign IDs fail. The server overrides the
client shape on non-client edges with horizontal/vertical/diagonal resize roles,
using the same four-pixel edge geometry as dragging. Original bounded 24px vector
artwork is a declared substitution. Hotspots remain the device hit-test position;
clipping and old cursor restoration handle negative artwork origins at output edges.
An operation rejected with Closing may race a queued CloseRequested. Clients drain
that event instead of recreating the window; a revoked channel still fails POLL
and reconnects. A failed save can cancel close and resume painting on that window.

The software renderer composes cached content, clipped rounded chrome, contour,
shadows and antialiased Selawik glyphs in stacking order. Damage covers old and new
bounds and shadow/pop-up coverage. Full private-frame redraw/copy is currently
conservative; moving a cached window does not ask its client to repaint. CPU-copy
feedback means the checked GOP writes completed. Vblank, page flip, VSync, refresh
timestamps, hardware cursor, acceleration and tear-free output are unavailable.

The BSP timer captures i8042 bytes into a 511-entry effective SPSC queue, preserving
raw receipt timestamps independently of rendering. The display owner alone decodes
keyboard make/break/modifiers/ASCII and mouse buttons/motion. Overflow or hardware
input error invalidates the display input epoch; service restart resets decoders
and queue. Text console input/painting is suspended while the display is owned;
serial diagnostics remain available. Display-owner retirement restores the kernel
terminal. Device ownership is single-vCPU in the pinned Q35/TCG profile.

Historical P11 bounds: eight process slots; 32 handles/process; eight Sections up to4MiB
each; eight windows and eight registered buffers globally; four windows/four buffers
per owner; 6MiB charged bytes/owner and10MiB globally (registrations plus snapshots);
64 queued events/owner; output1024Ãƒâ€”768. CPU frame storage and transient capture fit
the separate16MiB server heap. Exhaustion returns bounded errors. Idle scenes do
not redraw. Broader hardware, fonts, DPI, IME, accessibility and full desktop
fidelity are not implied by this foundation.

Closure requires two real protected clients, move/resize/overlap/focus/minimize/
restore, exact saved bytes across boots, hostile protocol/buffer/lifetime cases,
client and service recovery, repeated resource accounting, full P00Ã¢â‚¬â€œP10 regressions
and an independent critic bound to the unchanged qualifying report.

P12 negotiated extension: HELLO accepts VERSION followed optionally by featurebits2 (8bytes); the reply adds CAP_TRANSIENT_DISMISS2 when selected. Legacy4byte negotiation retains its original event set. Event9 DismissTransients(window) cancels app-local popups on trusted nonclient interaction; desktop activation clears actual focus/capture. This extension is development-tested separately from historical P11 qualification.

P12 epoch-safe submission extension: featurebit4 advertises16byte COMMIT(window,
configure_epoch), while the original8byte COMMIT remains accepted. The HELLO
reply capability word is at byte20. Known requested feature bits2|4 are negotiated;
unknown bits fail Unsupported. The SDK can connect to legacy services for legacy
operations; commit_epoch returns Unsupported without capability4. Current native
workspace apps require that capability before CREATE. A stale epoch is rejected
before attachment lookup, so resize between ATTACH and COMMIT is recoverable.
WouldBlock retains an accepted attachment: producers retry COMMIT without another
ATTACH or overwriting shared bytes. A model changed during this retry stays dirty
for a fresh submission after release. Authority reservations are canceled only
when actual configure epochs change; failed/no-op geometry operations preserve them.

P12 band submission extension (2026-10-22): the HELLO reply always advertises
CAP_BAND_COMMIT 256 (a server capability, never requested). A 24-byte COMMIT
(window, configure_epoch, top u32, rows u32) names the only rows that changed since
the producer's previous accepted commit of the same buffer. When the window's
snapshot came from that buffer with the same descriptor, the server copies just
those whole rows, validates them, computes exact damage by comparing them with the
snapshot, and patches the snapshot in place; rows outside the band keep bytes that
were already validated, so a wrong producer claim can only leave that producer's
own previous pixels on screen. Without such a snapshot, or for bands above 8 MiB,
the server captures the whole buffer as for a 16-byte COMMIT. rows=0 or a band past
the surface height is InvalidRequest. Per-row translucency keeps the opaque flag
exact for occlusion. The native SDK compares its private frame with the section
(raw copies, no shared slices), writes only the changed rows, and falls back to a
full write after any failed submission or descriptor change.

The current development desktop composes the pinned background below clients and
a trusted48px switcher overlay above them. Maximization uses the reserved work area.
Switcher presses retain generational WindowIds, and overlay pointer/key input does
not reach clients underneath. Existing capture and active drags retain their sequence.

## P12 development extensions

SET_THEME0x5016 accepts one little-endian u32(0 light/1 dark), no transfers. Only
Init or an authenticated peer in the currently bound session can apply it. It
changes trusted title bars/taskbar; clients keep ownership of their content palette.
Settings persists the per-user Theme through Storage before applying it. Init reads
that user's setting with GET_SETTING_FOR0x2012, an eight-byte UID/setting request
accepted only from immutable Init authority. Other callers receive Denied.

Init application DESKTOP0x1113 accepts one command byte:1 Files,2 Notes,3 Settings,
4 shutdown,5 reboot,6 freeze admission,7 cancel/unfreeze. Only the kernel-authenticated current window-server incarnation
in system session0 can enqueue it. No transfers or arbitrary mode/path are allowed.
Init owns/reaps up to three singleton native GUI children; a separate probe slot
belongs to the adversarial image. Resource exhaustion rejects launch without
terminating the desktop. Closing all ordinary children does not end the session.

Native SESSION_QUERY0x612(handle,output24) requires QUERY on a live Session grant
and returns id:u64,epoch:u64,uid:u32,reserved:u32. IDs/epochs are the same current
nonwrapping session identity. Complete output is captured through checked-copy.
Revoked grants, reduced grants and unrelated object kinds fail. No launch authority
is granted by this query.

Development output supports up to3840x2160 physical pixels and32MiB per section,
with native host modes, explicit1280x720 and retained1024x768 diagnostic capture profiles. Dynamic
resolution changes are not supported. Widescreen qualification is separate from
historical fixed-coordinate P11 evidence.

The desktop uses64MiB per owner and192MiB globally for registered backing plus
retained snapshots; the current development server heap is256MiB and app heaps64MiB. Three3840x2080
content surfaces charge191,692,800 bytes. The256MiB heap covers their snapshots,
one replacement capture, output/wallpaper caches and text/metadata; shared producer
pages remain separately owned. Dedicated256MiB heap alias strides and32MiB section
strides occupy disjoint supervisor arenas. The supervisor pool has512 pages and
private roots256 pages with fixed multiword bitmaps. Native ELF files are bounded
to16MiB with unchanged16MiB mapped span and8segments.

BIND_SESSION optionally accepts32 bytes: the original24 bytes, theme:u32 at24,
then four zero bytes. Legacy24-byte calls keep default appearance. Init includes
the persisted user theme in this single bind, avoiding a reverse RPC dependency.
DESKTOP command0 returns80 bytes: requested power:u32(0 none/1 off/2 reboot),
admission-frozen:u32, then three24-byte app records(id:u32,reserved:u32,pid:u64,
incarnation:u64). Empty records are zero. The broker obtains each identity from
the authenticated child channel. Window association never depends on mutable
caption text. The display polls this one-way catalog and performs graceful close
before sending final power command4/5. A successful cancel-close aborts shutdown;
hung closing clients retain the existing bounded termination policy (outside a
shutdown CAP_WIN32 clients follow "Win32 close and not responding"). Init does
not call back into Window while servicing its desktop requests.

A saturated client queue does not discard a power/close request. Window records a
fixed300-tick deadline even when CloseRequested initially returns QueueFull, retries
notification as the queue drains, and preserves queued input. Repeated close clicks
do not extend this deadline. Expiry uses authenticated Init termination and ownership
revocation; responsive save failure cancels the pending power operation.

Shutdown obtains command6 acknowledgement and a fresh post-freeze catalog.
Freeze cancels queued unadmitted launches and rejects new1..3 requests. Live
catalogued children must reconnect/create and process CLOSE or reach bounded
containment; an empty scene alone does not authorize session revocation. Cancel
save/close sends7. Failed control delivery retains fixed retry deadlines. Expired
owners are quarantined by PID/session/incarnation until termination acknowledgement
or NotFound. Window restart aborts incomplete freeze and preserves app processes.

Device input RESET kind3 has zero payload fields except timestamp. A BSP IRQ-off
raw queue overflow/parity reset clears the queue and decoders. Window cancels
focus/capture/drag/press/menu/shortcut state, establishes the next mouse button
baseline and continues. Lost input is not synthesized. Taskbar hover without an
open Start panel uses an overlay-only repaint and48-row transfer over cached
scene pixels; it does not recapture apps or recompose wallpaper/windows.

## Client caption and bounded application activation (P12)

HELLO capability bit8 enables CREATE role4, an ordinary TopLevel with a full-height
client surface. The previous parent field contains max control width:u32 and side
margin:u32. Width must be1..2048; margin138..2048. The centered interactive title
band shrinks with the window. The first32px allow a Back control; system buttons
retain higher-priority hit regions and trusted drawing, and other caption space
remains draggable. Default captions retain32px content inset. Configure, attach,
input coordinates, rendering and resize all use the selected content rectangle.
Window IDs and all existing ownership/epoch/quota checks remain authoritative.

LAUNCH_APP0x5017 accepts exactly one u32 catalog ID1..3 from the active user session.
It activates an existing singleton or asks trusted Init to queue it. IDs0/4/5/6/7
and unknown IDs are rejected; this request cannot power off, freeze admission or
change session identity. It is rejected while shutdown is pending. Init DESKTOP
continues accepting only the current Window manifest.

Close containment starts at service-processing time, not the original hardware
input timestamp, which may be queued behind composition. The300tick bound remains.
Opaque snapshots use clipped row copies, while transparent captures keep source-over.
Fully covered windows/shadows may be culled only behind a square opaque maximized
window. Client feedback and snapshot lifetimes retain existing semantics.

Voluntary RELEASE_CAPTURE acknowledges release without queuing CaptureLost;
involuntary focus/capture changes still notify. A delayed acknowledgement event
must not cancel a subsequent pointer press.
Applications drain accepted keyboard and pointer model events in FIFO order through
CLOSE even if an intervening paint/capture RPC reports Closing. New capture/paint
is suppressed; an accepted button pair retains its local press state. Window refuses
new routing to a closing window. Notes saves only after these accepted edits drain.

## Immutable offline desktop resources

Desktop initfs includes system/desktop-assets.blob, independent of Window ELF.
DOORART1 has16-byte magic/count/exact-length header followed by four16-byte
records (ID,offset,length,reserved:u32). IDs0..3 and contiguous ordered extents
are mandatory. Reserved/mapped page padding is zero; checked extent <=32 MiB.
Every payload is an opaque3840x2400 RGBA QOI. The archive authenticates its bytes.
The loader fills backing before publishing a READ|MAP|QUERY-only Section in
StartupV2 slot6, only to Window. No WRITE/TRANSFER/DUPLICATE grant exists.
Normal mapping leases retire it on process exit/restart; failed publication rolls
back. Ten global Sections cover the resource plus eight graphics buffers and one
transient/recovery allocation; existing graphics quotas remain unchanged.
Window validates the grant and pack, tests denied write mapping and maps read-only.
Wallpaper changes decode/validate first, then reuse the old output allocation;
three retained4K clients do not require a second full destination wallpaper.
SET_APPEARANCE0x5018 accepts appearance0..7: low bit dark, remaining bits wallpaper.
SET_THEME0x5016 still accepts only0/1 and preserves the selected wallpaper.
Legacy24/32-byte BIND_SESSION retains its layout and theme-only behavior.
The40-byte bind extends it with wallpaper:u32 at32 and zero reserved bytes36..40.
Settings saves the pair in one durable transaction; Init reapplies it at bind/restart.

Desktop shortcut gestures use single selection and a500ms double-click bounded
by four pixels of movement. A drag, input reset, different target or unrelated
activation cancels the pair. Start/taskbar commands remain single-release actions.
The native Window loop queues at most one pending launch per catalog app and
nonblockingly polls Init admission/catalog responses with a fixed500tick timeout.
LAUNCH_APP acknowledges bounded Window queue admission; actual process admission
still belongs to Init and failures are diagnosed. Session/shutdown checks apply
before queueing. Freeze clears pending launches and discards any pre-freeze catalog
request before establishing fresh post-freeze identity. No client receives Init
authority. CPU scanout reads an exact-serial immutable transfer view; input can
damage the next scene while render/repaint stays blocked until completion.
Choosing a desktop mode updates the shared Theme0/1; legacy text Theme2 remains
readable and is replaced only on an explicit appearance change. Scale is preserved.


## Current P12 shell refinement (development, not phase qualification)

Outputs at least2560pixels wide use192DPI/200% logical geometry; narrower
declared modes use96DPI. QHD2560x1440 has1280x720 logical units rasterized at2x.
HELLO reports physical extent and actualDPI; app CREATE/CONFIGURE/input transport
remains physical. Clients convert layout/input at the boundary and rasterize fonts,
ICOs, glyphs and preview assets at native density. No completed frame is enlarged.
Engine caption height/hit geometry uses the same scale. Default visible launch
remains windowed1280x720. Dynamic display mode/DPI changes are not implemented.

Large captures use four36,864,000-byte cells allocated before clients. Three may
remain retained; one remains available for replacement or wallpaper decode. Publish
returns the prior allocation, and destroy/disconnect return all descendant captures.
Failed capture/decode preserves the prior scene. Small captures use fallible
allocation; declared graphics quotas do not promise residency under heap pressure.
The4K CPU profile permits a fixed1000tick close grace; other modes retain300ticks.
An existing close deadline is never extended. Software transfer remains non-vblank.

CAP_APPEARANCE=16 opts into coalesced APPEARANCE_CHANGED=10 notifications at POLL;
legacy clients never receive them. Latest state survives until polled, independent
of the bounded ordered input queue. HELLO seeds current appearance. Application
control/text roles use dark/light palettes; color bitmaps bypass theme remapping.

DISPLAY_CLOCK0x824 requires the sole Display grant and Window manifest. It captures
16-byte validated UTC civil time; reserved bytes7..16 are zero. Kernel validates the
complete destination before device access. QEMU MC146818 reads use bounded stable
samples with UIP/BCD/12h/24h/date validation under the IRQ-off device guard; no CMOS
data is written, RegC is not read. Invalid/unavailable time is reported explicitly.
Tray calendar uses Gregorian real dates; disconnected/muted indicators describe the
no-NIC/no-audio declared profile. No battery/network/audio state is fabricated.


Trusted Init DESKTOP command8 admits the development user session only after the
current kernel-authenticated Window incarnation requests sign-in. It is session0,
no handles, one byte, and rejected after admission/power transition. The trusted
input context handles Enter/Space or a complete Sign in button pair before clients
exist; input RESET and session binding cancel pending press state. This is a
passwordless development gate, not credential authentication or a secure provider.

Capture copies service input between128KiB batches and revalidate closing/epoch
before publication. Native64px CUR frames/hotspots are selected at200%. Caption
hover and press repaint only caption damage without notifying the client. Commands
act on release over the original target; reset/drag-off/revocation cancels gestures.
The previous presented flyout extent survives until its dismissal frame completes;
closed flyouts require only48logical rows instead of580. The cached underlay remains
large enough to restore any supported flyout. Full software composition still costs
CPU; these rules imply no frame-rate or vblank guarantee.

### Trusted session placement (development)

Window assigns at most8 generation-checked workspaces per session. Organization
is not an authorization domain. Trusted switches revoke focus/capture. Owned
transient descendants move with roots. Removing a desktop merges records without
killing clients or discarding snapshots/close deadlines. Recycled IDs become stale.
Launch intent is captured before asynchronous admission and applied before CREATE
activation. Bundled apps are global singletons. Wallpaper is shared; desktop
organization is not durable. Snap retains the original normal work-area rectangle;
Closing rejects placement. Held Alt+Tab freezes MRU ordering and activates on
selection/Alt release. Bounded private previews cache accepted commit revisions.
No client ABI or producer-memory borrowing is introduced for global shortcuts.

Development diagnostics emit OVERVIEW_PRESENTED only on mode transitions after
successful CPU transfer and cursor output. The mode belongs to the rendered frame,
captured before interleaved input can change gesture state. This does not observe
vblank. Regression capture waits for a new marker and actual card/selection pixels;
the retained PNG is an exact encoding of its asserted PPM, not another screendump.


The current development shell accepts physical Win+A/Win+N for the existing
quick-settings and calendar/notification flyouts. Chord releases are swallowed;
competing overlays, focus and capture are revoked together. These are existing
truthful absent-device/empty-notification surfaces, not new device services.
Passwordless sign-in draws a native96/192px ContactSolid mask inside a192logical
avatar; one shared Layout owns the painted and trusted button hit rectangle.


Start/tray return focus is a saved exact generational window, owner, workspace and
session binding. Escape/same-toggle restores only a visible normal/maximized origin
with matching current identity; there is no arbitrary fallback. Explicit choices,
RESET, rebind, revocation and workspace transitions clear it. Outside pointer
dismissal passes the same press to the actual target. Blank disabled flyout areas
retain the flyout. Dismissal swallows Escape release before restoring app input.

## Desktop controls (P12 development)

CAP_DESKTOP_CONTROLS=32 negotiates Controls and coalesced event11, a=validated
six-bit value, other payload fields zero. SET_DESKTOP_CONTROLS0x5019 accepts
exactly4LE bytes and no transfers, with authenticated bound owner/session/epoch;
trusted Init may restore preferences during admission. Bits0..1 edge,2 start
alignment,3 compact,4 inline context actions,5 Properties bottom; default48,
unknown bits rejected. Storage Setting3 persists this separate per-user HKCU64
DWORD without changing the existing disk schema. Legacy clients receive no events.

LAUNCH_APP retains its4-byte app request; negotiated clients may send8bytes
app3,page15(Taskbar) or16(Context menu). Other app/page combinations are rejected.
SETTINGS_NAVIGATE event12 delivers the requested page only to the broker-classified
Settings owner; it cannot redirect another application. Pending latest controls
are independent of the bounded ordered input queue. Live reflow validates every
configure/restore/queue plan before mutation, retires changed attachments and
closes overlays while restoring only a validated saved focus origin.

Geometry, persistence failure behavior and unsupported controls are documented in
[Desktop controls](../../docs/design/desktop-controls.md).

## P12 client document modal and canceled Close

The shared app-modal control is client-owned and consumes that window's normal
input until an explicit choice completes or is canceled. It does not confer a
new secure/child-window role. Dirty or uncertain Notes Close first CancelClose;
Save uses the checked Storage transaction, discard writes nothing and Cancel
retains RAM/history. Accepted close blocks later editing until CLOSE arrives;
window recreation restores a visible prompt if the prior approved close was
interrupted. Save/reload failures retain the intent and model.

Close/Closing cancels an uncommitted server attachment without transferring
producer ownership or issuing BufferReleased. Clients retire only that local
pending hint; they preserve actual availability. After CancelClose they also
acknowledge the current configure epoch again, draining stale/new configure or
Close events normally. This prevents a stale hint from skipping ATTACH and
causing NoAttachment on the resumed repaint. This bounded client integration
does not qualify injected native Storage failures or full server modal policy.


## Terminal native preview extension (22 October 2026)

Desktop command 9 launches Terminal using Workspace startup mode14. Existing power4/5, freeze6/7 and signin8 meanings remain. Catalog0 now returns104 bytes: its unchanged8-byte header and four24-byte records, app identities1 Files,2 Notes,3 Settings,4 Terminal. Catalog identity4 and launch command9 are distinct. SDK and Init/Window are rebuilt together; historical80-byte consumers are not compatible with this development catalog extension. App indices0..3 own workspace and pending-launch slots; the independent adversarial client has a fifth tracking slot. Admission freezing cancels pending Terminal launches and denies new ones through shutdown. No shell engine availability or CMD/PowerShell/WSL compatibility follows from a catalog entry. See docs/design/terminal-native-preview.md.

Tabbed client captions extend the existing trusted native frame with role5 and negotiated CAP_TABBED_CAPTION64. CREATE packs physical interactive-width in the low16bits and logical title height32..72 in the high16bits of its first parameter; the second parameter is physical left inset. SET_TABBED_CAPTION0x501a updates only owner-checked tabbed windows and preserves their title height. Trusted46DIP minimize/maximize/close controls always win hit testing; blank title remains draggable. The server caches a reduced, blurred selected-wallpaper material for premultiplied tabbed client pixels, invalidating it on wallpaper replacement. Normal client composition remains unchanged. Wallpaper-backed translucent windows cannot reuse translated composition pixels. This is a development extension, rebuilt together with its native SDK.

## Wheel, acrylic material and window transitions (22 October 2026, development)

Device ABI (`door_abi::graphics`): DisplayInfo capability WHEEL=8 is reported when
the kernel negotiated a PS/2 IntelliMouse during display acquisition (sample rates
200/100/80, then GET ID: ID 3 four-byte IMPS/2, ID 4 IMEX with a four-bit wheel;
any other ID, or a NAK during the probe, keeps the standard three-byte protocol).
A MOTION event carries the signed wheel delta in `code` (two's complement,
-127..=127 detents, positive = away from the user); `Input::wheel()` reads it and
decode rejects an out-of-range value. Consumers that ignore `code` see the original
three-button motion event. Packets are decoded with the negotiated size after every
input-epoch reset.

Window protocol additions (CAP_WHEEL is requested in the HELLO feature word and
echoed only when granted; CAP_ACRYLIC is server-advertised to 8-byte HELLOs):
- CAP_WHEEL=512, granted only when the display reports WHEEL. INPUT detail
  INPUT_WHEEL=4 carries the content-local physical pointer position in rect.x/y and
  signed detents in `a` (two's complement). The shell sees the wheel first (open
  Start/flyouts/Task View or the taskbar consume it); otherwise it is routed to the
  pointer capture owner or the window under the pointer, without activation. A
  burst merges into the owner's newest queued wheel event for the same window
  (saturating at 127) without crossing any other queued event; with a full queue the
  wheel is dropped (counted as a dropped notification) instead of revoking the owner.
  An active window drag swallows the wheel; a window whose owner did not
  negotiate CAP_WHEEL receives nothing (legacy event sets are unchanged).
- Pointer leave (no new capability; WM_MOUSELEAVE equivalent): when the window
  receiving pointer input changes (to another window, the desktop, shell UI such as
  the taskbar, Start, flyouts or Task View), the previous window receives one INPUT
  detail 2 (motion) event at content-local (-1, -1), outside its client area. It is
  sent once per crossing, never to the pointer capture owner (a captured drag keeps
  receiving real motion and is left only after release, if the pointer is then
  elsewhere), never to a destroyed or closing window, and never changes focus or
  activation. It supersedes that window's queued motion (at most one queued motion
  per window); when the owner's queue is full it is dropped and counted as a dropped
  notification instead of revoking the owner. Older clients see an ordinary
  out-of-client motion.
- CAP_ACRYLIC=1024 and SET_MATERIAL=0x501d: 12 bytes (window u64, material u32), no
  transfers, owner-checked (Closing windows return Closing). MATERIAL_DEFAULT=0 keeps
  the theme content color (tabbed captions keep the cached wallpaper frost);
  MATERIAL_ACRYLIC=1 composes the window's translucent pixels over a live blur of the
  scene composed beneath it in the same frame, captured before its own shadow (never
  the window itself or windows above it): 8-DIP area downsampling, two separable
  radius-2 box passes, bilinear sampling, sampled reach = window + 40 DIPs. Damage
  within that reach recomposes the whole reach; caption hover/press near such a
  window uses normal damage; it never culls windows beneath it. Changing the
  material causes no epoch change or client repaint. Desktop Controls
  REDUCE_TRANSPARENCY replaces tabbed and acrylic materials with an opaque fallback
  (243,243,243 light / 32,32,32 dark).
- (27 October 2026, development) MATERIAL_MICA=2, accepted under the same CAP_ACRYLIC:
  Windows 11 Mica behind the window's translucent client pixels and its trusted title
  bar. The material is the wallpaper grid (16 px area cells, the frost above) blurred
  further by three 9-cell box passes per axis, moved to the luminosity of the theme tint
  and covered by it (dark #202020 at 80 %, light #F3F3F3 at 50 %), computed once per
  wallpaper and theme change and sampled bilinearly. Inactive windows and
  REDUCE_TRANSPARENCY show the opaque fallback above. No wire change beyond the value.

Server composition policy (no wire change):
- Transitions honor Controls REDUCE_MOTION (off disables them and completes running
  ones). A new top-level window is drawn invisibly until its first committed frame
  (at most 400 ms) and then scales 95%->100% while fading in over 200 ms
  (decelerate). Close and minimize take the window out of input immediately and keep
  a private image (ghost) that fades and scales out over 150 ms, or shrinks toward the
  app's taskbar button over 250 ms; restore and a cancelled close animate back from
  the current pose. Ghosts hold no client resources or quota, are bounded (6 images,
  64 MiB) and are freed when finished. Owned popups are not animated. Transitions only
  re-sample retained pixels: clients are never asked to repaint for them.
- Snap layouts: trusted `Engine::snap_window_rect(owner, id, zone, drag_restore)`
  places an owner-checked top-level window (not minimized or closing) at any zone
  inside the work area, such as the 2/3 + 1/3, half + stacked and quadrant zones,
  and marks it snapped exactly like the built-in halves/quarters (`snap_window`).
  The pre-snap normal geometry is kept: `drag_restore` when the title drag started
  from a restored size, otherwise the window's current normal geometry (its saved
  restore rectangle when already snapped or maximized). A caption drag restores that
  size under the pointer at the pressed title fraction, and unsnap returns to it.
  Zones outside the work area or below the minimum window size are InvalidGeometry.
  Every snap that changes geometry glides over 200 ms (decelerate) from the old
  bounds to the zone using the retained pixels; input targets the zone at once.
  Until the client commits a frame at the new size, its last frame is drawn
  stretched to the new bounds (at most 2 s after the glide) instead of exposing
  the material/background in newly uncovered area. Reduced motion snaps instantly.
- Up to four separate damage regions are composed and transferred per frame
  (overlapping or cheaply mergeable rectangles merge); the taskbar overlay runs once
  per frame. Interactive resize configures only the newest geometry, once per frame
  and on release; consecutive motion packets with unchanged buttons are merged before
  hover, hit-test and drag work, preserving every button, key and reset in order.

## Shell layout persistence and Start file search (23 October 2026, development)

Per-user shell layout (`door_service_protocol::desktop::ShellLayout`), persisted as
two HKCU DWORDs beside DesktopControls (storage Setting 4 `ShellLayout` flags and
Setting 5 `ShellOrder`; both zero is the Windows default, every app pinned in catalog
order). Flags: bits 0..5 taskbar-unpinned mask, 6..11 Start-unpinned mask, 12..13
Start All view (0 Category, 1 Grid, 2 List; 3 rejected), 14 desktop icons sorted by
name, 15..31 zero. Order: bits 0..9 Start pin order and 10..19 taskbar pin order, each
the Lehmer rank (< 720) of a permutation of the six `DESKTOP_COMMANDS` apps (pinned
apps first in pin order, then the rest ascending), 20..31 zero. Storage validates both
words (`Setting::registry` checks) and rejects anything else.
- Read at sign-in: Init reads both values with GET_SETTING_FOR after BIND_SESSION and
  sends SET_SHELL_LAYOUT=0x501e (8 bytes: flags, order LE; no transfers) only when
  the layout is not the default. The window server accepts it only from Init in
  session 0 and applies pins, Start view and sort without writing back.
- Write path: the shell queues the new layout after a pin/unpin/move/view/sort change
  and sends Init application op SHELL_LAYOUT=0x1115 (8 bytes). Init accepts it only
  from the authenticated current display service (manifest WINDOW, session 0, the
  current window instance, a signed-in session, no transfers), validates it again and
  its desktop step writes both values for the session user with storage
  SET_SETTING_FOR=0x201a (trusted Init only; 12 bytes uid, setting 3/4/5, value).
- Desktop menu "Show desktop icons" writes through the same broker:
  DESKTOP_CONTROLS_PERSIST=0x1116 (4 bytes, validated Controls bits). Init stores
  Setting 3 for the session user, then applies it with SET_DESKTOP_CONTROLS, so every
  CAP_DESKTOP_CONTROLS client (Settings included) receives CONTROLS_CHANGED and the
  shell has no session-only copy. One Init request is in flight at a time; a failed
  write logs `DOOR_P12 PROFILE write failed` and leaves the visible state unchanged.

Start file search host (the shell stays storage-free):
- Init starts one windowless workspace process per signed-in session in launch mode
  `door_abi::platform::mode::SEARCH_HOST`=15 (StartupV2 accepts modes 1..=15) once
  the session is bound, outside the adversarial window-test image. It logs
  `DOOR_P12 SEARCH_HOST LAUNCHED pid=`, is restarted with backoff after an exit (at
  most 3 times, `DOOR_P12 SEARCH_HOST EXIT pid= code=`) and is retired by Init before
  the display service at power-off/sign-out with exit code -150 (Init waits for it).
  Task Manager lists it under Background processes as "Search indexer". It holds no
  windows, documents or preference files; File Explorer is not a provider.
- DESKTOP_STATE_V2 carries it in slot `SEARCH_HOST_SLOT`=6 (count 7) after the six
  app slots; the shell ignores that slot for app classes and uses it to authenticate
  the provider.
- CAP_SEARCH_PROVIDER=2048 in the HELLO feature word, echoed only to workspace
  clients (stripped for others). The shell uses the newest such client only while it
  is the catalogued search host, connected and not revoked; otherwise Start shows the
  honest row "File search isn't available right now" (its action opens File
  Explorer) and Recent keeps its empty state.
- Event SEARCH_QUERY=14 (a = serial), posted about 60 ms after the Start query stops
  changing; every changed query takes a new serial and replaces an unsent one.
  SEARCH_QUERY_GET=0x501f (4 bytes serial) returns the trimmed UTF-8 query (at most
  128 bytes) for the current serial only (otherwise NotFound); provider only.
- SEARCH_RESULTS=0x5020, provider only, one result per call because RPC bodies are 240
  bytes: `[serial u32][state u8][count u8][index u8][kind u8][name_len u8]
  [path_len u8][name][path]`, or `[serial][state][0]` for no results. state 0
  BUILDING, 1 COMPLETE, 2 PARTIAL (index stopped at a bound); kind 0 file, 1 folder;
  count <= 8; name 1..=128 bytes; path (the containing folder in C:\ form) <= 102
  bytes. Results for any serial but the current query or Recent serial, or after
  Start closed, are dropped. Index 0 starts a new result set; out-of-order entries are
  ignored.
- Start lists query results under "Documents" and "Folders" after Best match, Apps and
  Settings (All and the Documents filter), then "Search in File Explorer". With no
  best match Start shows "No results" above the File Explorer row and Enter does
  nothing until the row is chosen with Down. A file's actions are "Open" and "Open
  file location"; a folder's is "Open".
- Event SEARCH_RECENT=18 (a = serial), posted each time Start opens unless Recent or
  "Show recently opened files" is off: the provider answers with SEARCH_RESULTS for
  that serial, at most `search::MAX_RECENT`=6 items from the user's
  `door_workspace::recent` list (`/users/<uid>/.door-recent`), newest first. Start
  shows them in two columns (file-type icon, name, folder).
- Event SEARCH_OPEN=15 (a = query or Recent serial, b = index, detail 1 = "Open file
  location"). The provider decides: text and source files open in Notes, images in
  Preview, folders, other files and "Open file location" in File Explorer; a Recent
  item opened in File Explorer is recorded again (Notes and Preview record their own
  opens). Event SEARCH_EXPLORE=16 (a = serial): File Explorer's This PC search for
  the query. Event SEARCH_RECENT_REMOVE=19 (a = serial, b = index): "Remove from
  list" (`Recent::remove` + `save`); the shell drops the item at once and asks for a
  fresh list.
- OPEN_DOCUMENT=0x5021 (search host only): app command u32 (1 File Explorer with an
  encoded `explorer_search::Open`, 2 Notes or 10 Preview with a Door path) then
  1..=`MAX_DOCUMENT`=224 UTF-8 bytes (NUL only in Explorer requests). The shell
  launches or activates the app (a running File Explorer opens a new window) and
  posts DOCUMENT_OPEN=17 (a = bytes, b = 1 when the app was started for it) to the
  app's broker-classified CAP_DESKTOP_CONTROLS client; the app reads the payload once
  with DOCUMENT_GET=0x5022 (empty body; NotFound for anyone else or after 20 s).
  Notes opens the file in a new tab (or selects the tab showing it), Preview opens it,
  File Explorer shows the location or search (in place when started for it). When
  Notes or Preview cannot be started the host falls back to File Explorer at the
  file's location. Event kinds up to 19 are valid on the wire.
- Host (`door_workspace::shell_search`): one `explorer_search::Index`, advanced 4
  Storage listing calls per iteration in the background and 32 while someone searched
  in the last 3 s; rebuilt in the background for a query once older than 15 s and
  otherwise every 2 minutes; polls every 10/20/50 ms (searching/indexing/idle).
  Answers are ranked (exact name or stem, prefix, word start, substring; shallower and
  shorter first), sent once per serial and re-sent at most every 250 ms while the
  index grows. Names only, no contents.

Snap Assist (window server only, no wire change): after a window snaps into a zone of
a snap layout (flyout, title drag to the snap bar, Win+Z), to an edge or corner by
drag, or with Win+Left/Right, the layout's next empty zone shows thumbnails of the
other windows on the desktop (most recent first, at most 8; scaled copies of the
engine snapshots, as in Task View) with app icon and title. Click or arrows + Enter
snap the chosen window there (`snap::builtin` halves/quarters or
`Engine::snap_window_rect` with `snap::zone_rect`, restoring a minimized window
first) and move on to the next empty zone. Esc, a click outside the zone, any other
key, Start, flyouts, Task View, locking or switching desktops dismiss it. The cards
slide in over 250 ms (none with reduced motion).

Shutdown with unsaved documents (window server only plus one request; Windows 11):
- A CANCEL_CLOSE received while a shutdown is in progress means the app is asking
  its user about unsaved work. The shutdown waits for the answer: the app's pending
  close is cleared (no close timeout), its windows are not closed again and it is
  not terminated; Init stays frozen. Outside a shutdown CANCEL_CLOSE only restores
  that window, as before.
- Save or Don't save: the app closes its own window (REQUEST_CLOSE; not logged as a
  new `CLOSE REQUEST`, it continues the shutdown) and exits; the shutdown then
  finishes by itself without another request.
- Cancel: the app sends CLOSE_DECLINED=0x5023 (8 bytes, its window id; owner
  checked). If the shutdown was waiting on that app it is cancelled
  (`DOOR_P12 SHUTDOWN CANCELLED pid=`), Init is unfrozen and the session stays
  usable; apps that had already closed stay closed. Otherwise it is a no-op.
  The workspace runtime sends it when `Application::take_close_declined()` is set
  (Notes: Cancel or Esc in a whole-window close prompt).

## Shared application resources and per-application executables (ADR 0018)

Workspace applications are separate executables over the `door-workspace`
library. Manifest `WORKSPACE` (12) is unchanged; the kernel selects the image by
launch mode: DESKTOP `bin/door-files.elf`, SESSION `bin/door-notes.elf`,
SETTINGS `bin/door-settings.elf`, TERMINAL `bin/door-terminal.elf`, SEARCH_HOST
`bin/door-search.elf`. Any other Workspace mode fails launch with NotFound. App
identity remains (manifest, mode) for every consumer (window-server classes,
taskbar, Start, Task Manager, Init accounting). New application manifests, if
ever needed, use IDs 16 and up.

Desktop initfs includes `system/app-assets.pack`, format DOORAST1
(`door_initfs::app_assets`), little endian:

| Offset | Size | Field |
|---|---|---|
| 0 | 8 | magic `DOORAST1` |
| 8 | 4 | entry count n, 1..=4096 |
| 12 | 4 | exact pack length L, <= 32 MiB |
| 16 | 16 n | entries: key offset, key length (1..=128), data offset, data length (u32 each) |
| 16+16n | | keys, contiguous in entry order, then data, contiguous in entry order |

Keys are ASCII `[A-Za-z0-9._/-]`, without leading or trailing `/`, `//` or `..`,
strictly increasing bytewise. The data ranges end exactly at L; bytes after L
(page padding) are zero. A pack violating any rule is rejected as a whole.

In DESKTOP mode the kernel validates the pack before Init starts and, on the
first launch of a Workspace (non-search), Preview or Monitor process, copies it
once into a zero-padded section owned by an executive resource pin. Each such
process receives StartupV2 inherited slot 7 (`APP_ASSETS`): kind Section, rights
exactly QUERY|READ|MAP. WRITE, DUPLICATE and TRANSFER are never granted; a
READ|WRITE map request is AccessDenied. Other manifests and the search host get
an empty slot. Applications validate kind and rights, map the section read-only
at `0x7E00_0000` (top 32 MiB of the user section window; surfaces use
`0x7200_0000..0x7E00_0000`), parse the pack and keep the view for the process
lifetime. Lookups return the stored bytes; an absent key is empty. The pin is
released only after every process is reaped at platform shutdown, then the
section retires like any other; there is one physical copy per boot. The
section uses one of the 32 global section slots.

## Fractional display scaling (W8, spec §57; 23 October 2026, development)

This section supersedes the fixed 96/192/288 DPI note above for clients that
opt in. Host-tested; guest/visual qualification is pending (DISPLAY-01).

- Scale is a percent: 100, 125, 150, 175, 200, 225, 250 or 300. Logical to
  physical is `px(l) = floor((l x percent + 49) / 100)` (round half down).
  Rectangles snap from their logical edges, so abutting logical rectangles
  tile physical pixels with no seam or overlap. Lengths measured from a
  physical edge (caption buttons 46 DIP, borders, shadows, glyph cells) use
  the same rounding. The logical output is the largest extent whose snapped
  edge fits (`Scale::logical_len`): 2560x1600 at 150% is 1707x1067. Up to one
  logical pixel at the right/bottom edge stays wallpaper only.
- Effective scale: the stored preference (`ScalePercent`, bound by Init) when
  this output offers it, else automatic. Automatic is the host scale hint
  (DisplayInfo capability bits 32..47, from QEMU fw_cfg `opt/door/scale`) when
  offered, else the integer heuristic (2x from 2560 px wide, 3x from 3840).
  A scale is offered when it keeps at least 1024x600 logical pixels and the
  shell has a font atlas for it (today 100/125/150/175/200/300; 225 and 250
  have no exported atlas and are not offered).
- `CAP_DISPLAY_SCALE` (HELLO feature bit 12): HELLO reports DPI = 96 x
  percent / 100, and SCALE_CHANGED (kind 20) arrives first after HELLO and on
  every change: `a` = effective percent, `b` = DPI, `rect` = physical output,
  `value` = preference << 32 | recommended percent. It precedes the
  CONFIGURED events that keep each window's logical size. Clients without the
  bit see 96 x the nearest whole factor (exact halves up: 150% -> 192) and no
  scale events. Their windows are still configured at the new physical size.
- `SET_DISPLAY_SCALE` (0x5024, 4 bytes, 0 = automatic) from Init (session 0)
  or a workspace session client. An unknown or unoffered percent is refused
  with InvalidRequest. The reply is the effective percent. A change rescales
  the live desktop (`Engine::rescale`: every window keeps its logical
  geometry; maximized and snapped windows refit the new work area). It also
  reloads the shell atlas, rebuilds the taskbar material and underlay, and
  closes Start, the flyouts, Task View and Snap Assist. Buffers attached at
  the old size are dropped, and no application restarts. In-flight
  transitions answer WouldBlock. The server never writes settings.
- `DISPLAY_SCALE_INFO` (0x5025, no body): 16 bytes, preference, effective,
  recommended (what automatic resolves to) and the offered mask (bit i =
  `DISPLAY_SCALES[i]`).
- `SET_TIME_LOCALE` (0x5026, 8 bytes: time zone id u32, regional format id
  u32 from `door_locale`) from Init (session 0, the user's `TimeZone` (7) and
  `RegionalFormat` (8) settings at bind when not the defaults) or a workspace
  session client (Settings, live). Unknown ids are InvalidRequest. The taskbar
  clock, lock screen and calendar flyout then show the RTC time converted to
  that zone (IANA-derived DST rules) in the region's short time/date, calendar
  headings and first day of week; "UTC" marks the header only for the default
  zone. Serial: `DOOR_P12 TIME_LOCALE zone= region=` and, at each clock reset,
  `DOOR_P12 CLOCK UTC ...` followed by `DOOR_P12 CLOCK LOCAL zone=<iana>
  region=<tag> <date> <time>`. The server never writes settings.
- Shell assets at fractional scales: text uses exported hinted atlases
  `hinted-125/150/175.dfa` (1,070,084 bytes, embedded in door-window only,
  decoded by `FontAtlas::from_packed_percent`). Segoe Fluent masks, cursors,
  caption glyphs, app/file icons and wallpaper previews area-filter the next
  whole density's native frame. A native frame of the exact physical size
  is used when one exists. Soft shadows sample the next larger integer
  texture.
- Diagnostics: `DOOR_P12 DENSITY physical=WxH logical=LWxLH factor=F
  percent=P hint=H` at start (F is "2" or "1.5"), and `DOOR_P12
  DISPLAY_SCALE preference=.. effective=.. dpi=.. logical=..x.. factor=..` on
  each live change.

On-demand read-only resources (ADR 0018 addendum). The desktop initfs carries
`door_abi::platform::resource::PATHS` (whole Windows fonts). `RESOURCE_OPEN`
(0x306, one argument: id 1..=7, other arguments zero) from a Window, Workspace,
Preview or Monitor process returns a QUERY|READ|MAP Section handle; any other
caller gets AccessDenied, an unknown id InvalidArgument, an absent file
NotFound. The first request copies the file into one zero-padded section held
by an executive resource pin (at most 8 pins: the pack and seven fonts);
later requests share it. READ-only views may be mapped in
`resource::VIEW_START..VIEW_END` (`0x8000_0000..0x9000_0000`) as well as the
section window; writable views stay in the section window. Pins are released
after every process is reaped at shutdown. Slot 7 (`APP_ASSETS`) is also
granted to the Window manifest, which maps the pack read-only at
`0x7E00_0000` for its font set.

## Win32 window relations (P13, development)

CAP_WIN32 = 8192 is requested in HELLO and echoed by servers that implement
the operations below; clients that do not request it keep the exact
native behavior. They exist for the Windows GUI personality (user32), which
keeps HWNDs, classes, children and window procedures in the application and
maps visible top-level, owned and popup windows onto native windows.

- SET_OWNER 0x5027, 16 bytes: window u64, owner window u64 (0 clears). Both
  windows must belong to the caller; self and cyclic ownership are
  InvalidRequest. Destroying an owner leaves its owned windows unowned.
- SET_ENABLED 0x5028, 12 bytes: window u64, enabled u32 (0/1). A disabled
  window receives no pointer press or key input and its caption buttons do
  nothing; a press, key or caption action on it activates its most recently
  related enabled owned window, followed down the ownership chain (a modal
  dialog, or a message box above that dialog). Disabling the focused window
  hands focus to that modal window. The decision never calls into a client.
- MOVE_RESIZE 0x5029, 24 bytes: window u64, edges u32 (0 move, else
  EDGE_LEFT 1 / EDGE_TOP 2 / EDGE_RIGHT 4 / EDGE_BOTTOM 8, never both
  opposite edges), reserved u32 zero, press serial u64. It starts the same
  trusted interactive move or border resize as a caption/border drag, only
  while the primary button that produced the INPUT press event with that
  timestamp on that window's client area is still held and no drag runs;
  otherwise StaleInstance. Client-drawn chrome answers its own non-client hit
  test (WM_NCHITTEST in user32) and continues the press through this call;
  the server never asks a possibly hung client to hit-test.
- SET_SIZE_LIMITS 0x502a, 24 bytes: window u64, minimum width/height u32,
  maximum width/height u32 (outer pixels, 0 maximum = unbounded, minimum
  above maximum = InvalidRequest). Interactive resizes (anchoring the edges
  not dragged) and CONFIGURE requests are clamped to them.
- SET_FRAMELESS 0x502b, 12 bytes: window u64, frameless u32 (0/1), top-level
  only. A frameless window (user32 sets it for every window without
  WS_CAPTION: popups, menus, client-drawn chrome) has no trusted caption
  buttons, resize border or title bar; every pixel is client area.

Server behavior for Windows GUI clients (P13 guest runs): a trusted move of a
manifest-15 window ends with a CONFIGURE carrying the final rectangle (user32
tracks positions for GetWindowRect and WM_EXITSIZEMOVE); destroying the
focused window, or its client exiting, activates the owner of an owned popup,
else the next visible top-level window (as close requests already did); and
Alt+Tab with no active window starts at the most recent window.

Relations are bounded (64 records) and released on DESTROY and disconnect.
Unicode titles are not part of this extension: SET_TITLE remains printable
ASCII, so user32 sends an ASCII rendering and keeps the exact UTF-16 title
for GetWindowTextW. Host coverage: `door_window::win32` tests (cycles,
modal chains, anchoring, press admission, bounds) and the service-protocol
opcode/capability disjointness test; guest qualification is pending.

CAP_WIN32 is granted only to Windows-personality processes (kernel manifests
WINDOWS_GUI 15 and WINDOWS_APPLICATION 10, `door_window::win32::grant`); any
other client gets it masked out of the HELLO reply and SET_OWNER,
SET_ENABLED, MOVE_RESIZE, SET_SIZE_LIMITS, SET_FRAMELESS and SET_RESPONDING
are Unsupported for it (26 October 2026, P13 critic).

## Win32 close and not responding (P13, Windows 11; 26 October 2026)

Native Door clients keep the close semantics above unchanged (Closing state,
CANCEL_CLOSE, CLOSE_DECLINED, the bounded 300/1000-tick containment). A
CAP_WIN32 client gets the Windows 11 behaviour (`door_window::not_responding`):

- Close (caption button, Alt+F4, Task View, taskbar) outside shutdown and
  sign-out queues one CLOSE event for the window, delivered by POLL/POLL_BATCH
  after the events queued before it. The window keeps its state and input: no
  close transition, no deadline, nothing is ever evicted for taking time.
  `DOOR_P12 CLOSE REQUEST pid=P at=T` is still logged. user32 acknowledges with
  CANCEL_CLOSE the moment its thread retrieves the event, then dispatches
  WM_SYSCOMMAND/SC_CLOSE (WM_CLOSE). A save prompt (MessageBox) may stay open
  indefinitely. If the window still exists when WM_CLOSE returns, user32 sends
  CLOSE_DECLINED (a no-op outside a shutdown).
- Hung rule: a client is not responding when events have waited for it for
  five seconds (500 ticks) without a POLL/POLL_BATCH. SET_RESPONDING 0x502c
  (12 bytes: window u64, responding u32 0/1) lets user32 report the same for
  one window whose thread stopped retrieving messages for five seconds with
  messages pending while another thread still polls. A not-responding window
  is a ghost: the server keeps composing its frozen last frame under a
  whitened veil (premultiplied white at 50 %) and draws " (Not Responding)"
  after its caption title. Serial: `DOOR_P13 WIN32 NOT_RESPONDING pid=P
  window=W` when a window becomes a ghost, `DOOR_P13 WIN32 RESPONDING pid=P
  window=W` when it retrieves again.
- Close on a ghost opens the server's prompt instead of queueing another
  CLOSE: "<title> is not responding", "If you end the task, you might lose
  information.", buttons End task and Wait (Wait focused). Esc is Wait,
  Enter/Space the focused button, Tab and Left/Right move the focus; a click
  on a button answers on release; presses outside the prompt pass through.
  Serial on opening: `DOOR_P13 WIN32 NOT_RESPONDING_PROMPT pid=P window=W
  end_task=X,Y,W,H wait=X,Y,W,H` (physical button rectangles). Wait logs
  `DOOR_P13 WIN32 WAIT pid=P window=W` and only dismisses it; End task logs
  `DOOR_P13 WIN32 END_TASK pid=P window=W`, revokes the owner and terminates it
  through Init (the existing authenticated retirement). The prompt closes by
  itself when the window responds again or is destroyed.
- After any revocation of a CAP_WIN32 owner the server logs `DOOR_P13
  OWNER_RELEASED pid=P windows=N surfaces=N buffers=N attachments=N bytes=N`
  with the owner's real remaining counts (all zero unless something leaked).
- A full event queue of a CAP_WIN32 client drops the input (Windows drops input
  beyond a full queue) instead of revoking the client.
- Termination happens only on End task, or on the native containment path at
  shutdown and sign-out: there every client, Win32 included, gets the native
  close (Closing state and deadline); user32's immediate CANCEL_CLOSE makes the
  shutdown wait for its answer, Save/Don't save continue it, Cancel
  (CLOSE_DECLINED) cancels it, and a client that never retrieves the CLOSE is
  revoked at the deadline (`DOOR_P11 CLOSE TIMEOUT revoked`).

Host coverage: `door_window::not_responding` tests (a slow WM_CLOSE is never
timed out, a dirty-close MessageBox survives ten minutes, a real hang becomes a
ghost at exactly five seconds, a hung thread reported by user32, End task as
the only terminating answer, pointer and keyboard prompt input) and
`door_window::win32` capability-grant tests.
