# Windows 11 reference surface inventory

Target checked 2026-10-18: Windows11 26H2, build26300.9711. See the companion
reference packet for release sources, pinned WinUI/Gallery commits and asset
licenses. These rows are an acquisition checklist, not claims that corresponding
Door shell/apps already exist or match Windows. Controlled Windows captures are
**not acquired**. Public documentation/illustrations do not replace those frames.

Freeze edition, account, app package versions, feature rollout, DPI/text scale,
accent/theme, wallpaper, transparency, motion, language and taskbar choices.

| Family | Layout and controls | Required samples | Gate |
|---|---|---|---|
| Desktop/taskbar | Wallpaper crop, icon grid, placement/size/alignment, running indicators, tray | Empty/populated; launch/minimize/restore/overflow | P14 |
| Start/Home | Pinned/Recent/All, category/grid/list, search, account/power areas | Sections shown/hidden, responsive sizes, keyboard launch, errors | P14 |
| Search | Query, categories, result rows, preview | Partial/typo, local files/apps/settings, no results | P14 plus index/search |
| Quick settings/tray | Toggles, sliders, expansion, overflow | On/off/disabled, device unavailable, persisted values | P14/P15 |
| Notifications/calendar | Grouping, date/time, calendar grid, badges/actions | Empty/populated/error/dismiss, controlled clock | P14 |
| Window chrome | Title/icon, caption controls, contour, radius/shadow, hit regions | Active/inactive, all edges/corners, min/max/restore, hung close | P11 behavior; P14 fidelity |
| Switcher/desktops/snap | Thumbnails, selection, preview, layout flyout | Alt+Tab directions, cancellation, focus/capture handoff | P11 foundation; P14 shell |
| Explorer Home | Tabs, command/navigation/address/search bars, side navigation, quick/recent areas | Empty/populated/loading; account-specific rows separate | P14 |
| Explorer folder | Details/list/grid/compact, columns, breadcrumbs, selection/status/preview | Open/back/up, sort/scroll, keyboard selection, access errors | P14 |
| Explorer commands | New/cut/copy/paste/rename/delete/share, sort/view/more, modern/extended context menus | Name conflict, progress/cancel, durable reopen | P14 plus Storage |
| Settings | Home/categories/navigation/search/cards, expanders, switches/sliders/combos | Expanded/selected/disabled/error, saved values | P12 controls; P14 app |
| Editor | Tabs/menus/commands, text/selection/caret, find/replace, settings/status | Open/save/reopen, dirty close, save error, undo/redo, Unicode/IME | P12/P14 |
| Terminal | Tabs/profiles/title/menu, output/input selection and scroll | Process launch/exit/cancel, persisted settings, clipboard | P14 |
| Dialogs | Open/save/confirmation/progress, text/options, button order | Default/cancel, validation/error, modality/reentrancy | P12/P13/P14 |
| Trusted surfaces | Lock/logon/elevation, identity, credential/error controls | Secure routing, cancellation, failure/fallback | P14 plus security gates |
| Accessibility | Contrast/scaling/text size/focus contours/semantic tree | Keyboard-only, reduced motion/transparency, screen reader | P12/P14 |

Capture normal, hovered, pressed, focused, selected, disabled, loading, empty and
error states wherever applicable. Include capture/cancellation and focus return.
Record effective and physical coordinates separately; measure baseline/advance/
line height independently of font size. Capture motion and interruption with
timestamps. Layout, raster, interaction, motion and accessibility are separate
outcomes. Dynamic text/time must use the same controlled or narrowly masked data.

Primary layout references: [File Explorer support](https://support.microsoft.com/en-us/windows/experience/fileexplorer/file-explorer-in-windows),
[Explorer keyboard/accessibility layout](https://support.microsoft.com/en-us/accessibility/windows/use-a-screen-reader-to-explore-and-navigate-file-explorer-in-windows),
[spacing](https://learn.microsoft.com/en-us/windows/apps/design/basics/content-basics),
[button behavior](https://learn.microsoft.com/en-us/windows/apps/develop/ui/controls/buttons),
and the pinned control dictionaries in windows11-reference.md. Official
[Notepad notes](https://learn.microsoft.com/en-us/windows-insider/release-notes/apps/notepad)
describe Insider packages; pin the stable installed package separately.

P11 supplies two real native consumers. Command geometry is shared by painting
and hit testing; licensed outlines and open Fluent icons replace placeholders;
every visible command has a real operation. P12/P14 extend this foundation without
decorative controls that pretend to change absent services.
