# Application interaction reference and implementation

Reference checked 2026-10-18. This is the behavior backlog for the protected native
consumers, coordinated with the surface inventory and Windows reference packet.
It is not a declaration that these consumers are complete Explorer or Notepad.

Primary references:

- [Explorer layout, focus order, app bar and selection](https://support.microsoft.com/en-us/accessibility/windows/use-a-screen-reader-to-explore-and-navigate-file-explorer-in-windows)
- [Windows and Explorer keyboard commands](https://support.microsoft.com/en-au/accessibility/windows/keyboard-shortcuts-in-windows)
- [Explorer Home/navigation behavior](https://support.microsoft.com/en-us/windows/experience/fileexplorer/file-explorer-in-windows)
- [Notepad package updates](https://learn.microsoft.com/en-us/windows-insider/release-notes/apps/notepad)
- [Notepad formatting toolbar and status toggle](https://blogs.windows.com/windows-insider/2025/05/30/text-formatting-in-notepad-begin-rolling-out-to-windows-insiders/)
- [Microsoft mouse interaction conventions](https://learn.microsoft.com/en-us/windows/win32/uxguide/inter-mouse): single-click selects desktop shortcuts, double-click opens; Start commands use one click. This older Windows guide establishes behavior, not current Windows11 pixel styling.
- [Text and app sizing](https://support.microsoft.com/en-us/accessibility/windows/make-text-and-apps-bigger): display scale and text size are distinct from resolution.

Explorer's documented organization is tabs, navigation/address/search toolbar,
command bar, navigation/content panes and status controls. Notes needs the chosen
Notepad package's tabs, File/Edit/View menus, settings, editing/status controls and
explicit plain-text/formatting configuration. Insider announcements identify
references to acquire; they do not pin the installed stable app or its rollout.
Installed Windows11 Pro25H2 build26200.9711 references and app-package identities
are recorded in windows-core-apps.md and project/windows-app-inventory.json.
These are owner-configured references, not a clean-install inventory or a complete
per-page/state corpus. Desktop control stopped on the owner's Escape.

| Interaction | Current native consumers | Remaining shared toolkit/app work |
|---|---|---|
| Window commands | Alt+Tab/Shift+Alt+Tab, Alt+F4, Ctrl+W, caption controls, resize edges | System menu, snap/switcher surfaces, additional trusted shortcuts |
| Files navigation | Alt+Up parent, Alt+Left/Backspace back, Alt+Right forward; bounded history, address editing and name filtering committed after successful Storage listing | Tabs, full indexed search and the current Explorer toolbar order |
| Files commands | Ctrl+Shift+N new folder; F5/Ctrl+R refresh; F2 checked durable rename; bounded independently accepted Shift+Delete permanent removal with default Cancel; arrows/Home/End/PageUp/PageDown; Enter opens | Connection recovery, Recycle Bin/trash/restore/copy/move, multiselection, complete details/view behavior |
| Notes | Ctrl+S durable save, guarded Ctrl+O reload, Ctrl+A, directional Shift selection, Ctrl+Z/Y/Shift+Z, measured pointer placement/visible-row drag, visual wrap/vertical/page navigation, word/line/document movement and deletion, bounded ASCII/tab/CR/LF document; checked4096-byte durable stream replacement and separate save attention; real Save/Don't save/Cancel close/reload modal | Shift-click/edge autoscroll/horizontal scrolling, clipboard, find/replace, Unicode/IME, atomic namespace creation/Save As/conflict-resolution UI, tabs and full menus |
| Pointer feedback | Arrow, text I-beam, four resize directions; clipped hotspots and restoration | Busy/unavailable/drag states, text placement/selection, wheel/gestures |
| Buttons | Shared paint/hit geometry, release activation, hover/press, capture cancellation | Retained controls, Tab/Space/Enter, semantic tree and theme/scale states |
| Context menus | Shared command menus, right-click/Shift+F10/Menu, arrow selection, Enter/Escape, focus/nonclient dismissal | Item-specific/edit menus, complete native popup lifecycle and semantic controls |
| Assets |25 pinned Fluent SVGs, eleven Windows ICOs, six CURs, Bloom and local Segoe UI; Selawik default helper retained | Remaining app artwork and measured reference calibration |
| Motion/layout | Correct final composition and state, conservative CPU redraw | Reference-calibrated geometry/materials/motion, reduced-motion policy and timed captures |

Ctrl+N is deliberately not repurposed as new-folder. Implementing its real new-window
behavior requires coordinated lifecycle/launch support. Menus must invoke the same
typed commands as shortcuts and buttons; do not render unsupported commands as
working actions. A pointer hand is not used for ordinary desktop command buttons.

P11 qualifies native authority/composition. P12 owns reusable text/controls/focus,
clipboard, semantics and menu state machines; P14 owns complete application and
shell layout/calibration. Every new state needs host model tests and an actual
guest interaction with independently observed effects. Pixel, interaction, motion
and accessibility outcomes remain separate; none is currently Windows 1:1 qualified.

P12 initial refinement adds actual right-click command menus in Files/Notes, Shift+F10/Menu-key invocation, Up/Down/Enter/Escape, outside-click/focus/resize dismissal and shared command IDs. Files uses a separate navigation/address and command row, back/forward/up controls, and requested pinned folder/Explorer/Notepad color icons. These are incremental development changes, not complete Explorer/Notepad layouts or measured fidelity. The CPU compositor skips opaque blending and fully occluded shadow interiors; GPU/VSync and Windows motion calibration remain unqualified.

Windows cursor resources now replace the original provisional vectors: arrow, text I-beam, horizontal/vertical and both diagonal resize cursors. The host exporter selects the exact32px frame and preserves hotspots, alpha and monochrome inversion; guest rendering blends over the immutable background and restores old clipped bounds. Sources/hashes are assets/cursors/windows/manifest.json. Trusted dismissal event9 cancels app popups for nonclient interactions; bare-desktop clicks clear focus/capture.

Notes development layout replaces the oversized Save/Reopen toolbar with a document header, File popup and a working word-wrap toggle. Menus use the same Storage operations and keyboard commands. Menu dismissal is negotiated (HELLO capability2), so legacy4byte HELLO clients never receive new event9. Current local assets are pinned Windows ICO/CUR references; no Windows1:1 claim or full P12 closure follows from this batch.
