# Native UI services: clipboard and drag and drop (v1)

Status: protocol, window-server broker and client library implemented and
host-tested (P12 W5, stage 1). App consumers (Notes, File Explorer, Terminal)
and the Settings > System > Clipboard page are stage 2. Requirements: master
§58 (clipboard, drag/drop, trusted interaction) and gate TEXT-01.

Both services ride the authenticated `door.window` channel (manifest 11): the
window server owns the pointer, focus and lock state, so it is the broker.
Owners always come from kernel channel metadata, never from payload bytes.
Requests from another session, or before a session is bound, are refused by
the window channel itself.

## Reservations

| Item | Value | Owner |
|---|---|---|
| Clipboard opcodes | `0x5100..=0x517F` (`clipboard::FIRST..=LAST`) | W5 |
| Drag and drop opcodes | `0x5180..=0x51FF` (`dnd::FIRST..=LAST`) | W5 |
| HELLO capability | bit 20 `clipboard::CAP_CLIPBOARD` (0x10_0000) | W5 |
| HELLO capability | bit 21 `dnd::CAP_DRAG_DROP` (0x20_0000); bits 22..=23 reserved for native-ui | W5 |
| Event kinds | `0x100` clipboard, `0x110..=0x114` drag and drop | W5 |
| Accessibility opcodes | `0x5200..=0x523F` (`accessibility::FIRST..=LAST`) | U |
| HELLO capability | bit 24 `accessibility::CAP_ACCESSIBILITY` (provider), bit 25 `accessibility::CAP_ASSISTIVE` (assistive client) | U |
| Event kinds | `0x120..=0x12F` accessibility (see "Accessibility" below) | U |
| Text input opcodes | `0x5240..=0x527F` (`text_input::FIRST..=LAST`, `specs/contracts/text-input-v1.md`) | I |
| HELLO capability | bit 26 `text_input::CAP_TEXT_INPUT`, bit 27 `text_input::CAP_IME` (IME manifest only) | I |
| Event kinds | `0x130..=0x13F` text input (`0x130 PENDING`) | I |

The events reuse the 64-byte window `Event` layout. `window::Event::decode`
rejects these kinds; the server sends them only to clients that negotiated the
matching capability, and `WindowClient::poll` decodes them for those clients
(`clipboard::decode`, `dnd::decode`). Window opcodes and event kinds below
`0x100` stay with `window.rs`.

## Clipboard (`door_service_protocol::clipboard`)

One clipboard per signed-in session, as in Windows.

**Copy-at-set (chosen over delayed rendering).** COMMIT validates each staged
format, then the server keeps a private copy. The reasons:

- A paste never calls back into the owner, so a request cannot run another
  process's code (§58), and a hung owner cannot block a paste.
- Owner death needs no special path. Windows likewise keeps non-delayed data
  when the owner exits, and asks a delayed-rendering owner to render everything
  before it goes. `Info.owner` changes to `OWNER_EXITED` and the data stays.
  No CHANGED event is sent, because the content did not change.
- Memory is explicitly bounded (see the size table below).
- The cost is a copy of large payloads. That is acceptable at these sizes, and
  `WRITE_SECTION` makes it a single snapshot.

**Staging:**

1. `BEGIN(window)` opens a staging set. The caller must own the window, and a
   new BEGIN replaces the caller's earlier staging.
2. `FORMAT(format, bytes)` declares each format with its exact size.
3. The bytes arrive in order through `WRITE(format, offset, ≤232 bytes)`, or in
   one `WRITE_SECTION(format, offset, bytes)` with a section handle (READ right).
   The server snapshots that handle and closes it; it is never mapped.
4. `COMMIT` replaces the clipboard and returns the new sequence. A COMMIT with
   no formats empties the clipboard. `ABORT` discards the staging.

Staging is discarded when its client exits or the session locks.

**Reading:**

- `INFO` returns the sequence (GetClipboardSequenceNumber), the format mask, the
  owner state and each format's size.
- `READ(sequence, format, offset)` returns up to 240 bytes. It answers `Changed`
  once the sequence moves on, so a multi-chunk read is consistent.
- Only the client that owns the focused window may READ, as Windows requires
  for packaged apps. Background apps cannot snoop. A Win+V paste is delivered
  to the focused app.

**Formats and limits:**

| Format | Windows | Check | Limit |
|---|---|---|---|
| `TEXT` 1 | CF_UNICODETEXT | UTF-8, no NUL | 8 MiB |
| `FILES` 2 | CF_HDROP | NUL-separated absolute Door paths, no control characters | 1024 paths × 1024 B, 256 KiB |
| `IMAGE` 3 | CF_DIBV5 | `[w][h]` then premultiplied BGRA (c ≤ a) | sides ≤ 8192, 8 MiB |
| `PNG` 4 | "PNG" | signature only (never decoded in the server) | 8 MiB |
| `HTML` 5 | "HTML Format" | UTF-8 fragment | 8 MiB |
| `DROP_EFFECT` 6 | "Preferred DropEffect" | COPY or MOVE (Explorer Cut = FILES + MOVE) | 4 B |

Other limits:

- One set is at most 8 MiB (`MAX_CONTENT`).
- Everything the broker holds counts toward 32 MiB (`MAX_BROKER_BYTES`): the
  clipboard, history, staging and drag offers. When that budget is reached, the
  oldest unpinned history items are evicted first; if that is not enough, the
  request fails with `Resources`.
- Files travel as paths only. Readers open them with their own identity, so
  file permissions are never bypassed.

**Locked session.** Every clipboard request is `Denied` while the session is
locked: no reads, no writes, no settings. Staged sets are dropped at lock.

**History (Win+V, Windows 11).** Off by default.

- The panel opens at the focused app's text caret (its door.text-input
  context caret): below it with left edges aligned, above it when the work
  area ends first, always inside the work area. Without a text caret it opens
  right-aligned above the taskbar. While history is off it shows the "Turn on"
  prompt; with history on and nothing copied, the "Nothing here" empty state.

- Text, HTML and bitmap items up to 4 MB are kept. History holds 25 items,
  newest first. When it is full, the oldest unpinned item goes.
- Copying an item again moves it to the top.
- Choosing an item makes it the clipboard (owner: shell), moves it to the top,
  closes the panel and pastes it into the app that had focus (Ctrl+V).
- "See more" offers Pin/Unpin and Delete. "Clear all" keeps pinned items.
- Settings "Clear clipboard data" (`CLEAR_DATA`) clears the clipboard and the
  unpinned history.
- Turning history off (`SETTINGS_SET`) clears it, pinned items included.
- `SETTING_SYNC` stands for "Clipboard history across your devices". Doors has
  no cloud, so pinned items stay on this device. History is in memory only in
  this stage.
- `SETTINGS_SET` and `CLEAR_DATA` are accepted only from workspace apps (the
  Settings app).

## Drag and drop (`door_service_protocol::dnd`)

This is the OLE DoDragDrop/IDropTarget model with scoped offers.

**Targets.** `REGISTER(window, formats, effects, default)` corresponds to
RegisterDragDrop: it lists the accepted format mask, the effects the window can
perform and its usual effect. `REVOKE` undoes it. Registrations end with their
window or client.

**Source:**

1. `BEGIN(window, allowed effects)` opens a drag staging. Formats are then
   staged exactly as for the clipboard.
2. `START` is accepted only while the primary button is held with the pointer
   over (or captured by) the source window, so a drag comes from the user's own
   gesture. It returns the drag serial.
3. The source loses pointer capture. The server owns the pointer until release
   or cancel.

**During the drag:**

- The server hit-tests windows on every motion. A registered target that
  accepts at least one offered format receives ENTER, OVER (coalesced) and
  LEAVE. Each event carries the content-local position, the accepted subset of
  the offered formats (never content), the allowed effects and the key state.
- The target answers with `STATUS(serial, window, effects, default[, insert])`,
  which can narrow but never widen its registration. The optional insert (up
  to `dnd::MAX_DESCRIPTION` = 64 UTF-8 bytes, no control characters,
  `dnd::valid_description`) is DROPDESCRIPTION's szInsert: the name of what
  is under the pointer (a folder, a document). A STATUS without it clears it;
  a new target starts without one.
- The effect is `dnd::resolve`: the target's default, Ctrl → Copy, Shift → Move,
  Ctrl+Shift or Alt → Link. It is always limited to (allowed ∩ target), and
  NONE means "can't drop here".
- The shell paints the effect next to the pointer as Explorer does: "Copy to
  %1", "Move to %1" or "Create link in %1" with the insert, else the bare
  "Copy", "Move" or "Create link". Nothing is shown while the effect is NONE.

**Taskbar.** Resting on a taskbar app button for 400 ms (the Windows hover time)
brings that app's window forward once per rest. The taskbar itself never takes a
drop.

**Ending:**

- **Esc**, the source's `CANCEL`, a lock or an input reset cancels the drag.
  The target gets LEAVE; the source gets `FINISHED(NONE, OUTCOME_CANCELLED)`.
- **Release with no willing target:** `FINISHED(NONE, OUTCOME_REFUSED)`.
- **Release over a willing target:** the target gets DROP carrying the resolved
  effect. Only that target may `READ(serial, format, offset)`, and only formats
  it registered for (others are `Denied`; other clients get `NotFound`).
- The target ends the drop with `FINISH(serial, performed effect)`. The
  performed effect cannot exceed the resolved one (NONE = failed). The source
  then gets `FINISHED(effect, OUTCOME_DROPPED)`.
- Unfinished drops expire after 30 s, and the source gets `OUTCOME_LOST`.

**Owner death:**

- A source that exits mid-drag cancels the drag.
- A source that exits after the drop leaves its copied offer readable until
  FINISH or expiry.
- A target that exits loses the drop, and the source gets `OUTCOME_LOST`.

**Locked session.** Every drag request is `Denied` while locked.

## Client library (`door_service_client::window::WindowClient`)

Clients request the capabilities with
`connect_with(registry, CAP_CLIPBOARD | CAP_DRAG_DROP)`.

| Clipboard call | Purpose |
|---|---|
| `supports_clipboard` | the server granted CAP_CLIPBOARD |
| `clipboard_set(window, &[(format, bytes)]) -> sequence` | replace the clipboard |
| `clipboard_set_with(window, inline, &[(format, section, bytes)])` | large formats through sections |
| `clipboard_info() -> Info` | sequence, formats, owner state, sizes |
| `clipboard_read(sequence, format, offset, out)` | one chunk of a format |
| `clipboard_read_into(&info, format, out)` | a whole format |
| `clipboard_settings`, `set_clipboard_settings`, `clear_clipboard_data` | Settings page |

| Drag and drop call | Purpose |
|---|---|
| `supports_drag_drop` | the server granted CAP_DRAG_DROP |
| `register_drop_target(window, formats, effects, default)` | RegisterDragDrop |
| `revoke_drop_target(window)` | RevokeDragDrop |
| `start_drag(window, effects, &[(format, bytes)]) -> serial` | start a drag |
| `start_drag_with(window, effects, inline, sections)` | large formats through sections |
| `drag_status(serial, window, effects, default)` | answer ENTER/OVER |
| `drag_status_described(serial, window, effects, default, insert)` | answer with a drop description ("Copy to Documents") |
| `drop_read(serial, format, offset, out)` | read dropped data |
| `drop_finish(serial, effect)` | report the performed effect |
| `cancel_drag(serial)` | source cancels its drag |

## Not yet provided

- **Server-to-client bulk reads.** These are chunked (240 bytes per call). A
  section-based READ is reserved for a later revision.
- **Persistence.** Pinned items and the history setting are not yet persisted
  across sign-in, which needs a storage path through Init.
- **Delayed rendering, cross-session clipboard and remote sync.**
- **Drag images.**
- **Drops onto shell surfaces.** Desktop icons, Start and the taskbar do not
  take drops.

## Door UI toolkit (`door-ui`, stream U)

Status: implemented and host-tested (P12 U, stage 1): 26 toolkit tests and 6
gallery tests. Apps still use the immediate-mode runtime; migrations are
stage 2. Requirements: master §49–51, §55, §113.

**Model.** `Ui` owns a retained widget tree. A `WidgetId` is an arena index
plus a generation, so a removed widget's id never resolves again, even after
its slot is reused. Only the UI thread mutates widgets. Background work uses
`begin_async(id) -> AsyncToken` and `deliver(token, payload)`. A result is
dropped (and counted in `FrameStats::async_discarded`) when the widget was
removed, its slot was reused, or newer work was started.

**Frame pipeline.** event → model update → invalidation → layout if needed →
paint if needed → semantics if needed. `render` replays the display list
into a private frame at any display scale. Damage is the union of the old
and new bounds of each widget whose recorded paint changed (focus rings
included), clipped to the window. Unchanged frames cost nothing.

**Layout.** Logical units are fixed-point (1/64 DIP). The kinds are stack
(gap, justify, flex weights), grid (Fixed/Auto/Star tracks, spans), overlay
and scroll, with min/max, margins, padding, alignment and baselines. In a
stack, weighted children share the free space and give it back when the
content overflows (star sizing), so a growing ScrollViewer gets the remaining
viewport rather than its content height.

**Input and focus.**

- Events route capture → target → bubble. Disabled subtrees receive nothing.
- Pointer press captures. Losing capture (deactivation, removal, disabling,
  a modal layer opening) cancels the press.
- Focus moves on press without the focus visual. Keyboard focus shows it
  and scrolls the control into view.
- Tab order: explicit `set_tab_index` first, then tree order. Hidden,
  collapsed and disabled controls are skipped, and `set_tab_stop(false)`
  removes a stop that stays focusable (WinUI `IsTabStop`).
- Focus scopes: a modal layer traps Tab and blocks input below it. Closing a
  layer restores focus to where it was. Removing the focused widget moves
  focus to the next stop (wrapping). Deactivating the window remembers focus;
  reactivating restores it.
- Menu mode (F10, Alt+letter on a MenuBar) returns focus to its origin on
  Escape or after a command, as in Windows.

**Popups.**

- Owner popups (ComboBox drop-down, menus) take pointer and keyboard input
  first while focus stays on their owner. While a context menu is open, its
  highlighted item is the semantic focus.
- A click outside dismisses the popup and is consumed. Escape also
  dismisses.
- Resizing or deactivating the window closes popups.
- Light-dismiss layers (flyouts) report `Notify::Dismissed`.

**Visual states.** Controls paint from immutable theme snapshots (light,
dark, the four contrast themes, text size 100–225%, reduced motion and
transparency). `force_visual_state(id, Rest | Hover | Pressed | Focus)`
changes paint only, as `VisualStateManager.GoToState` does. Behaviour, hit
testing and semantics keep the real state. The gallery's state matrix uses
it.

**Catalog.**

- TextBlock, Panel, Button (standard, accent, subtle, hyperlink),
  ToggleButton, CheckBox (two- and three-state), RadioButtons, ToggleSwitch,
  Slider, ProgressBar and ProgressRing.
- TextBox (single and multi-line), PasswordBox and NumberBox, over a
  document model.
- ComboBox, ListView (virtualized, single and multiple selection), TreeView,
  MenuFlyout, MenuBar, CommandBar, TabView, NavigationView.
- ScrollViewer, Expander, InfoBar, ToolTip, ContentDialog.

Each control has a state machine (activation as in WinUI `ClickMode.Release`;
Space arms and invokes on release; Enter invokes on press) and semantics, and
accepts the matching accessibility actions.

**Semantic updates.** `take_semantics` returns the full tree plus a delta
from the tree the previous call returned (`base`). Versions published in
between fold into that one update.

**Inspector.** `Ui::inspect()` reports this window only: identity, bounds,
layout kind, invalidation reasons, measure and paint cost, and semantic role.

**Native adapter (`door-ui-native`).**

- Hosts an `App` on the shared native runtime.
- `App::theme` lets an engineering fixture pin a theme.
- With the `accessibility` feature, the adapter publishes the semantic tree
  (full tree, then deltas from the version the broker holds) and performs
  brokered actions.

## Accessibility (`door_service_protocol::accessibility`, stream U)

Status: wired end to end (P12, 24 October 2026) and host-tested; guest
evidence comes only from the `component-gallery` and `a11y-01` suites.
Requirements: master §58, A11Y-01.

- **Wiring.** The protocol and client modules are declared;
  `WindowClient::poll` decodes events `0x120..=0x12F` for clients that
  negotiated a capability, and the native runtime routes the session-wide
  ones (TREE_CHANGED, FOCUS_CHANGED, ACCESS_CHANGED) to the first window.
  The window server dispatches `0x5200..=0x523F` into the broker
  (`graphics/window-server/src/accessibility_glue.rs`).
- **Grants.** CAP_ACCESSIBILITY goes to any session client that asks (the
  native runtime always asks). CAP_ASSISTIVE goes only to the kernel
  identity of manifest 17 (`ASSIST`, `door-assist`), whatever the request
  says.
- **Storage bounds.** Each provider may hold 16 trees and 16 384 nodes in
  total; a commit past either bound answers `Resources`. One staging
  buffer per client (at most 512 KiB).
- **Launch.** `door-gallery` (manifest 18, `GALLERY`) and `door-assist`
  are staged in both desktop archives and listed only in Start search, as
  "Door UI Gallery" and "Captions" (accessory commands 12 and 13). They are
  not pinned and have no taskbar button. Init starts each at most once per
  session and retires them at session end.

**Shell and app adoption.**

- **Shell trees.** The window server publishes the taskbar, Start, Quick
  settings, the Notification center and the input switcher under reserved
  window ids (`door_window::shell_semantics`), from the geometry that
  painting and hit testing use. These trees are built only while an
  assistive client is connected, and only when the shell state changes.
- **Captions.** The title bar and its Minimize, Maximize/Restore and Close
  buttons are merged into every top-level window's tree when it is read.
  Their Invoke runs the real caption action.
- **Shell keyboard.** Start's focus follows its keyboard model. Tray
  flyouts gain keyboard stops: Tab, the arrows, Enter and Space.
- **Immediate-mode apps.** These apps describe their controls through
  `door_workspace::semantics` (`Application::semantics`), using the layout
  their `target` hit-tests. The runtime then:
  - builds a validated tree (node ids from target ids; focus on the focused
    control, otherwise the window);
  - publishes it at most once every 3 ticks (full, then deltas);
  - clicks a node's target for Invoke, Select, Toggle, Expand and Collapse.
- **Shared descriptions.** Context menus, classic drop-down menus and
  message dialogs share one description each.
- **Coverage.**
  - Settings: rail, search, page cards, combo lists and menus.
  - File Explorer: tabs, navigation buttons, address, search, command bar,
    navigation tree, items view and Home sections, context menu, delete
    dialog. Tab moves between the items view and the navigation pane, and
    the pane gets tree keys and a focus ring.
  - Notes: tabs, menu bar and menus, formatting bar, the document as an
    edit (its first 2 KiB and the selection), save prompt.
  - Terminal: tabs, and the console as a document.
  - Preview: every `actions()` control.
  - Task Manager: navigation, commands, table rows, settings, menu, Run
    dialog.
- **Focus restoration.** Each app keeps its own focus model across
  deactivation and popups. When a popup closes or the window is
  reactivated, the broker announces the restored element.

**Tree.**

- One tree per window, with a version that increases on every published
  change.
- Each node carries: role, name, value, description, accelerator, states,
  actions, bounds (1/64 DIP), range (milli-units), position in set, level,
  selection and labelled-by.
- Roles follow UI Automation control types. Collapsed content is not
  published, and virtualized lists publish only their realized rows (each
  with its position in the whole set).
- `PROTECTED` (password) nodes never carry a value or selection. The toolkit
  clears them before encoding, the encoder rejects them, and the broker
  strips them again.

**Wire format (`door_accessibility::wire`).**

- Header: magic `DA11`, format 1, kind FULL or DELTA, window, version, base
  version.
- A delta lists removed node ids and complete added or changed nodes. It
  applies only to its exact base version; a mirror that sees a gap asks for
  a full tree.
- Limits: 4096 nodes, 1024 children, depth 64, name 512 B, value 2048 B
  (truncation is flagged `VALUE_TRUNCATED`), message 512 KiB.
- Decoding validates everything before anything becomes visible: one root,
  consistent links, reachability, no control characters in names, known
  roles and flags, ranges, selections on character boundaries, and focus on
  a live node.

**Providers (`CAP_ACCESSIBILITY`, bit 24).**

- `PUBLISH_BEGIN`/`CHUNK` (or `SECTION`) then `COMMIT` publishes a full
  tree or a delta for an owned window.
- `COMMIT` answers `Changed` when a delta's base is stale; send a full tree.
- `WITHDRAW` removes a tree before its window is destroyed.
- Assistive actions arrive as `ACTION` events. The provider reads the
  arguments with `ACTION_GET` and answers `ACTION_DONE` (done, rejected or
  gone). Disabled nodes and actions a node does not advertise are refused.

**Assistive clients (`CAP_ASSISTIVE`, bit 25).** This capability is granted
from the kernel-authenticated identity, never from request bytes.

- `FOCUS_GET`, `WINDOWS`, `SNAPSHOT` (scope tree, focus or node; at most 4
  frozen snapshots per client), `READ` (chunked) and `INVOKE` (at most 16
  pending actions per session).
- Events: `TREE_CHANGED`, `FOCUS_CHANGED`, `ACTION_RESULT` and
  `ACCESS_CHANGED`.
- Secure surfaces (sign-in, lock, elevation) read as a single placeholder
  node.
- A locked session denies every read and action, drops snapshots and
  pending actions, and sends `ACCESS_CHANGED(0)`.

**Clients in this repository.**

- `door-assist` (desktop/component-gallery) is a Narrator-style caption panel. It
  mirrors trees, announces focus, entered containers (dialog, menu, list,
  tree, tab list, menu bar), state changes and live regions.
- For guest checks it prints `DOOR_ASSIST CAPTION` lines and
  `DOOR_ASSIST TREE` blocks: the tree in `door_accessibility::outline` form,
  without geometry, focus or visibility.

**Gallery and guest suite.**

- `desktop/component-gallery` shows every page of the catalog, plus a state matrix
  (rest, pointer over, pressed, disabled, focused) for 17 interactive
  controls, with a Theme picker (light, dark, four contrast themes).
- Host tests:
  - render every page in four themes, and the densest pages at 100, 125,
    150 and 200%;
  - require each state to paint distinctly, on the same cell and at the
    same pixel phase;
  - compare stored semantic trees and a keyboard-only trace
    (`desktop/component-gallery/expected`; `DOOR_BLESS=1` rewrites them);
  - write renders as BMP files when `DOOR_GALLERY_RENDERS` is set.
- `cargo xtask test ui --suite component-gallery`
  (`tools/xtask/src/desktop/gallery.rs`) drives the guest gallery by
  keyboard. It compares each page's tree, as `door-assist` read it through
  the broker, with the host fixture, and checks focus captions, the toggle,
  password redaction and dialog focus restoration.

## Not yet provided (toolkit and accessibility)

- **Guest evidence.** Neither `component-gallery` nor `a11y-01` has run
  on the guest yet, and the size of the two staged accessories against the
  16 MiB loader limit has not been measured.
- **Adoption gaps.**
  - No Win+T taskbar keyboard traversal. Tray flyout stops have no focus
    ring.
  - Shell context menus, Task View, Snap and the Run and clipboard panels
    publish no tree.
  - Settings administration pages publish only the rail and heading.
  - Terminal settings pages publish only their heading. Preview and Task
    Manager describe controls but have no keyboard focus model to report.
  - Assistive Focus and SetValue on app nodes are refused.
  - Accessories have no taskbar button.
- **Screen reader.** Speech waits for audio (P15). Text ranges beyond
  value and selection, and a UI Automation bridge for Windows applications,
  are later work.
- **Scale and reference evidence.** Guest display-scale coverage comes from
  the W8 `display_scale` scenario; the gallery suite runs at 100%. The
  high-contrast pointer-over and pressed looks of subtle and hyperlink
  buttons are identical here, and not yet measured against Windows.
