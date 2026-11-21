# Measured text navigation and pointer selection

P12 development batch; host579 tests and both native26-check/two-boot Notes
workflows pass with unchanged inputs. The accepted Notes editing checkpoint
remains separately bound in notes-editing.md. Independent read-only review accepted
the bounded scope in reports/critics/p12-text-navigation-critic.json, SHA256
a4610d6350791d12dab8d1dfd5f7dbc5fb3304e44e5496d66b7499a92481515a.
This batch does not close P12 or establish Notepad1:1.

DocumentView is a shared measured editing controller, independent of native
Storage and painting. The caller supplies text geometry and font measurement.
Notes uses its actual logical20px left inset,86px text top and20px line height.
The same geometry controls visible rows and pointer hit targets. Hidden rows have
no pointer target; captured motion clamps to the visible first/last row. Focus
loss, capture loss, configuration and transient dismissal cancel a held drag.
The I-beam uses the same text target rather than separate approximate geometry.

A soft-wrap boundary has upstream/downstream visual positions at one byte offset.
Document history retains caret affinity along with the directional selection.
Visual Home/End and Left/Right visit the appropriate positions; Up/Down and
Page Up/Down retain a measured preferred column across short/empty rows. Shift
extends selection. Ctrl movement retains the existing logical word/document
operations. The original editor history, byte bounds and Storage checks remain.

Wrapped lines are retained by content revision, width and wrap state. Height-only
layout changes and selection movement reuse them; edits/undo/redo invalidate.
Loading another document or changing font metrics requires explicit reset. A
saturated content revision returns no cacheable identity, avoiding epoch alias.
Painting reads retained lines rather than repeating complete wrapping on hover.
Nearest caret search uses monotone measured ASCII advances with a binary search,
avoiding quadratic prefix walks on long unwrapped lines. Tests bound search work
and check every printable ASCII pair at1x/2x/3x for the actual selected font/style.

The [Microsoft TextBox reference](https://learn.microsoft.com/en-us/windows/apps/develop/ui/controls/text-box)
and acquired Notepad reference guide caret placement, selection, constrained
multiline geometry and wrapping. Door implements its own controller; Microsoft
UI, shaping, clipboard or accessibility implementations are not imported.

Native fixture extensions use actual mouse/key input, settled document-pixel
comparisons and private raw-disk checkpoints. They verify insertion at the clicked
position, mouse/keyboard line-selection agreement, replacement preserving LF,
capture cancellation, both wrap-caret sides and undo, page return, preferred
vertical column and selected-row deletion undo/redo. Original two-boot Notes,
theme, dirty-reopen and exact-byte checks remain.

The qualifying build is23084, report SHA256
05d30c1324df451e34d85ee20cd822b1c5f2e61e97a43d2deefd88f2179fb5ad.
Its source identity is3d8138320a2908a42f652ca483afebd55ca1ac4eb73c37e6217697956483733f.
Only the host Notes fixture's coordinate oracle changed afterward: Engine's
client origin is400,242, text origin420,328 and caret top420,327, with450DIP
wrap width. The earlier fixture incorrectly added a side/bottom inset. Failed
16152/19036 reports and screenshots remain retained; their product carets were
already correct. The final crop includes both actual wrap-caret positions, and
cancellation compares motion while the left button is still held, before release.

Final source identity isf6da8bef4412d43a7d21e63f67f6fdb50a1e4407835b28cecc8cf87136bb6c7a.
All following reports record unchanged inputs:

| Check | Report under reports/runs/ | SHA256 |
|---|---|---|
| Host579 tests/33 executables | host-1792539708096323300-12504/report.json | 3e2381b72efa2b5d745ee7ba751bc016a979a725e7e51ab51183da996395f864 |
| Notes1x TCG26 checks/two boots | host-1792539708130836400-22652/report.json | 77a116d01039224032c867503930cfc5e95d111178f71e60d4a2af1b438ba8b1 |
| Notes3x WHPX26 checks/two boots | host-1792539708138120900-10172/report.json | e0227616298fa66af89a2943953a59e864b9c3b90bc79babc74c6eb60481eca4 |
| Files1x19 checks/two boots | host-1792539836382228100-24488/report.json | c5f11ed2ef935cbbbf563fc366ab5245ffe83d09f415e95c742185d675025cc9 |
| Files3x19 checks/two boots | host-1792539836410062100-3976/report.json | 84e0c1a587c83da8c647ce601470661a46aca5b6cd3887f5009e55923fab093b |
| Default4K launcher/render/drag16 checks, original build source | host-1792539492528254300-26144/report.json | c7fbea00f80202665c5efcee3e7157a1f8df148063cbddd6d54d57a5562b60c7 |

Kernel, services, window server and other apps match22876/16216 byte-for-byte;
only Workspace changed. The earlier combined6840 (243 checks) and foundation25484
(42 outcomes) are inherited solely for those unchanged subsystems, not fresh
runs of the new consumer. Workspace's three PT_LOAD segments span11,337,728bytes,
below16MiB. No new dependency/version/feature/unsafe code was added.

Cumulative11340, SHA256fb33d50e99aad77adc343087fbda12ea1fa3598b043f3146728eade5fcbd0d70,
still fails on the original missing P00 report. It records unchanged final inputs;
historical identities are preserved rather than substituted.

Open: Shift-click, double/triple-click selection, edge autoscroll, horizontal
scrolling for unwrapped text, clipboard, Unicode/shaping/IME, full modern menus,
tabs/dialogs/accessibility and atomic save/conflict prevention. Visible-row drag
selection is not complete Windows text editing. No FPS claim is made.
