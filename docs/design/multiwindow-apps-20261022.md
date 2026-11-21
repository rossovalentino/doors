# Native multiwindow apps and document captions

22 October 2026. P12 implementation checkpoint; native qualification and independent
review are pending. This does not certify exact Windows fidelity or complete app
providers. The [master audit](p11-p12-master-audit-20261022.md) records P11/P12 scope.

## Window and document ownership

The six bundled app classes are Explorer, Notes, Settings, Terminal, Preview and
Monitor. Each process can retain four independent window models. A single
authenticated connection dispatches events by actual window ID. Each window owns
its private pixel buffer, shared-section mapping, epoch, input state and model.
Font atlases and the bounded icon cache are shared within the process. This is
multiple windows with concurrent model state, not a separate process per window
or unrestricted multithreaded application execution. A blocking Terminal command
can still delay other Terminal windows.

Explorer uses Ctrl+N for a new window and preserves Ctrl+Shift+N for a new folder.
The other apps use Ctrl+Shift+N. Shift-clicking a running catalog app requests a
new window. Notes uses Ctrl+N for another independent document tab. Every window
has separate close behavior; closing one does not retire its siblings.

Explorer and Preview share measured caption geometry, close targets and the
Explorer-style selected tab. Drag jitter below6DIP remains a click. A same-owner
caption can accept an owned document; foreign windows and protected title buttons
cannot. Cross-process and cross-app document transfer are not implemented.
Stable process-local document IDs prevent a reorder or intervening close from
redirecting a pending move to the wrong document.

For detach, allocate/register a blank destination and create its actual window;
retain the source tab until the destination's first successful commit. Only then
move the complete model and close an empty source. The five-second deadline,
explicit destination close or display reconnect cancels the unadmitted blank
window while retaining the source. Destination preconditions and image budgets
are checked before removal. Preview moves its owned pixels/text/metadata, scroll,
zoom, fit and undo image. Explorer preserves its directory/history/search/view/
selection/details snapshot and rebuilds cancelled read-only preview caches.

## Presentation and edit changes

Tabbed caption/rail surfaces now use premultiplied tint over the compositor's
existing cached wallpaper frost. This is wallpaper material, not live blur of
arbitrary windows behind the app. Opaque host fixtures remain an explicit fallback.
No GPU/vblank/input-to-photon claim follows from this material.

Notes has a40DIP integrated caption, File/Edit/View command row, centered formatting
controls and32DIP status footer. UI labels use Segoe UI; native editor layout,
selection and caret use the same Cascadia Mono atlas. Bold/italic/strike/link,
heading/list/table tools insert undoable Markdown source; this is not rich-text
rendering. Each document retains its own text, undo, checked save identity,
uncertainty/close guard and viewport. Window close prompts through every dirty
document and cancellation restores editable state. Open/Save As, clipboard,
find/replace, Unicode/IME and full modern Notepad functionality remain missing.

Preview now supports crop selection, explicit Apply/Cancel, and one-level reversible
crop/rotate/flip. Edits affect owned pixels; export creates a separate PNG/QOI copy.
Undo pixels count against the16MiB per-window image budget. Generated WAV waveform
or OBJ visualization pixels do not enable image editing. Adjustment/markup/erase,
PDF/Office renderers, audio/video playback, OCR/recording and broad conversion
providers remain unavailable; their decorative controls must not imply success.

Monitor reuses the actual display glyph for System and formats measured memory
with a compact GB value. Its ten destinations and CPU/Memory drilldowns retain
the previous real sanitized telemetry. Unavailable network/GPU/startup/service
control providers remain explicit; no sample hardware values or synthetic graphs
are added to resemble the concept. New windows copy presentation preferences.

## Changed foundations and required evidence

The previous eight-process/three-app capacity prevented all six app classes from
running together. The bounded development catalog now admits twelve processes,
with matching executive/VM arrays, disjoint kernel arenas,32sections/32channels,
64handles/process and256objects. An initial64channel trial overflowed a host test
stack; the failing log is retained, and32channels passed that focused regression.
Telemetry v2 carries12records in864bytes while608byte legacy requests remain valid.
The native monitor stays read-only and identity/session constrained.

Window capacity is8owners/24windows/32buffers, four windows/eight buffers per owner,
128MiB per-owner and192MiB aggregate registered/captured bytes. Workspace/Monitor
use96MiB heaps, Preview128MiB. These finite limits still reject combinations that
exceed available memory. Producer mappings never alias and grow/shrink allocation
failures preserve model state. New-source admission includes a working-memory
margin. Neither host allocation tests nor a tiny image qualify native1MP pressure.

Run a full `desktop-qemu-x64` build for these kernel changes. A desktop-development
relink using an older kernel is insufficient. The new private `test multiwindow`
workflow exercises Explorer/Preview detach and attach, distinct Notes save bytes,
separate windows of every app class and eleven live processes. It binds fresh
gesture byte ranges and presented window IDs to the finally retired native actors,
then independently audits the raw disk. Run it alongside applicable window
adversarial/recovery and app persistence/visual checks. Preserve failures.

Exact1:1 remains unqualified. The concepts contain app providers that do not yet
exist; compressed/resized references and variable telemetry are not pixel-equality
fixtures. Full light/dark, narrow, DPI, accessibility, keyboard and fault-injection
coverage remains work toward the open P12/P14 gates. The absent historical P00
and P11 run reports are not reconstructed. The owner volume is not used.
