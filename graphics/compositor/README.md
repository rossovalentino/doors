# door-graphics-core (P11)

This `no_std` + `alloc`, unsafe-forbidden crate is the portable CPU window
foundation. It has no third-party dependencies. It neither authenticates IPC nor
reads client-mapped memory. The service is the trust boundary for both operations.

## Integration

1. Construct `Engine::new(width, height, Theme::Light, limits)`. Dimensions are
   physical pixels, at most3840 by2160. Set the global and per-owner byte limits
   to fit the service heap. Registered backing bytes and private snapshots both
   count, even when they represent the same client image. The persistent output
   frame additionally costs `width * height * 4` bytes.
2. Derive `Owner { pid, session, epoch }` from authenticated kernel metadata.
   The epoch is a process/service incarnation, not a client-selected integer.
   A protocol decoder must not deserialize an Owner supplied by the client.
3. `create_window(owner, Role::TopLevel, outer)` returns a generational WindowId
   and queues Configure with epoch 1. OwnedPopup requires a live same-owner
   parent; privileged roles explicitly fail. Geometry describes the outer frame;
   Configure and WindowInfo give the content extent below the 32 pixel title bar.
4. Register a validated `SurfaceDescriptor` using `register_buffer`. Its public
   fields are revalidated on registration. `required_bytes` is the full padded
   stride times height; captures passed to commit have exactly `backing_bytes`.
   Associate the returned BufferId with the authenticated kernel section handle
   in the service's own table. That association must also use the generation.
5. The client acknowledges the current configure epoch and attaches a registered
   buffer of the exact content extent. `attach` reserves metadata for this window;
   it does not read pixels or transfer producer ownership. A newer configure,
   close, or destruction cancels that reservation. Only one reservation per
   window and buffer exists at a time.
6. Fault-safely capture the registered section into an owned `Vec<u8>` using the
   kernel. Supply that private capture to `commit(owner, window, captured)`.
   Commit validates premultiplication, dimensions, size, epochs, quotas and event
   capacity before atomically replacing the prior safe snapshot. On any error
   the prior snapshot and pending reservation survive. A successful commit queues
   BufferReleased: copying is complete and no subsequent render reads producer
   bytes. Unregistering that buffer does not invalidate the safe snapshot.
7. Move cached content with `move_window`; request resize with `configure`.
   Configure increments the window epoch, coalesces obsolete configure events,
   and requires a fresh acknowledgement and attachment. It retains the prior
   snapshot. Old pixels render at native scale, clipped to the new content area;
   newly exposed areas use the theme content fallback until a valid commit.
8. Drive rendering on `has_damage()`. Gather any borrowed WindowInfo/title data
   needed for decorations before calling `render()`. It returns a Frame borrowing
   a reusable private BGRA byte buffer. Add text through Canvas or the service's
   font renderer, respecting each title's clip/available caption width. Transfer
   this completed frame through the actual display backend, end the Frame borrow,
   then call `complete_transfer(serial, success, monotonic_timestamp)`.
   Failure dirties the full output for retry. Only one CPU frame is outstanding.
   CpuTransferred explicitly describes a successful CPU copy, with no claim of
   vblank, page flip, physical scanout completion, or tear-free display.
9. Feed trusted device events to `route_input`. Coordinates are output-global.
   It activates on real pointer down, follows capture for pointer events, and
   follows focus for keys. Hit testing uses the same outer/client/caption geometry
   and rounded silhouette as composition. The service implements drag/resize and
   caption actions using HitRegion and the owner returned by WindowInfo.
   `activate` and `cycle_focus` are trusted server policy operations; never expose
   arbitrary cross-owner activation as a client request. `cycle_focus(reverse)`
   includes minimized top levels and restores them, including their prior
   maximized state. `restore_window`, `request_state`, and `close` provide explicit
   state transitions. Close immediately disables input; destroy completes teardown.
10. On authenticated process death/session revocation, call `disconnect(owner)`
    and delete that owner's service section table entries. Every window, popup,
    pending attachment, snapshot, registration, focus, and capture is removed.
    Old IDs cannot address replacement objects. Reconnecting is explicit recreation;
    engine teardown does not itself revoke a kernel-authenticated identity.

## Bounded event semantics

Poll with the authenticated Owner. Configure keeps the latest epoch. Pointer
motion coalesces per window. Accepted key/button events and BufferReleased events
are reliable and consume bounded queue capacity. `QueueFull` means the operation
was not accepted: retain/retry device input in a bounded service queue, or revoke
that owner according to policy. In particular, never silently discard a key-up.

Focus/capture and latest-per-window presentation reports are advisory, so an
unresponsive event consumer cannot block the compositor or another application.
`dropped_notifications(owner)` changes when queue pressure drops an advisory;
reconcile using WindowInfo, `focused_window`, and `captured_window`. Frame requests
coalesce to one outstanding request per window. If their queue is full, they stay
pending; draining events makes the callback eligible again without an idle spin.
Invisible windows receive their pending callback when they become visible.

## Rendering and scope

The CPU renderer clips every draw to the output and uses integer premultiplied
source-over. Four by four corner sampling produces deterministic antialiased
8 pixel window rounding; maximized windows are square. Client snapshots, rounded
chrome, a contour, caption line artwork and bounded layered shadows share one
composition path. Damage conservatively unions old/new content and all affected
popup/shadow bounds, then clips to the output. Composition currently redraws the
whole private frame; the returned damage can restrict backend transfer. No
occlusion assumption discards underlying translucent content.

The light/dark colors are provisional original development tokens at physical
96-DPI reference scale. Their provenance is master section 140 bootstrap and
`docs/design/windows11-reference.md`, with the published title-bar/geometry
references in that file; profile is `P11 provisional 96-DPI`, confidence is
unmeasured. Title height is 32 physical pixels, window radius 8, shadow margin 12,
caption width 46, resize hit border 4. These values affect standard native window
chrome only. This crate makes no measured Windows fidelity claim.

P12 text shaping/rasterization, controls, accessibility trees, DPI negotiation,
clipboard, modal policy, child windows and arbitrary scene graphs are unsupported
here. P14 shell panels, taskbar, secure prompts and lock screen are unsupported
roles, rather than ordinary clients with special titles. GPU buffers, acquire
fences, zero-copy, animation, blur/Mica/Acrylic, output hotplug and scanout timing
remain unsupported. There is no fake desktop data or stock application fixture.

`cargo test -p door-graphics-core` exercises geometry/stride overflow, odd padding,
premultiplication, clipping, ownership/session/incarnation, stale generations,
rapid configure/ack/attach cancellation, snapshot and registration lifetimes,
queue/backpressure/key-up, failed-transfer retry, deterministic frames/allocation
reuse, popup damage, focus/capture and repeated cleanup. Host tests establish
portable behavior; guest IPC, kernel capture safety, actual display/input and
persisted application data need separate integrated guest evidence.

### Per-window title rendering

For font rasterization use `render_with_chrome(|info, canvas| { ... })` instead
of adding all titles after the completed scene. Each callback runs immediately
after that window's chrome/content/caption symbols, before upper windows and
shadows. `render()` remains the no-op callback wrapper. `canvas.dimensions()`
returns width/height/stride; `canvas.pixels_mut()` exposes the private frame to a
trusted rasterizer. `Engine::title_clip(info.outer)` excludes 12 pixel side
padding, 4 pixel vertical gutters and the three 46 pixel caption controls; a
window too narrow for that area returns None. Use that clip explicitly when
adapting to `door-graphics-text::Target`. Raw frame access does not enforce the
window clip for a trusted callback. A 14 pixel title line box starts at outer
x + 12, y + 6; content and caption controls must not be overpainted. Higher-window
occlusion remains correct because titles participate in scene composition order.
