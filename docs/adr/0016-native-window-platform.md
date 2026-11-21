# ADR 0016: Native window authority and captured CPU surfaces

Status: adopted; P11 cumulative qualification and independent critic passed.

The kernel owns the display grant, checked shared-section capture, i8042 input
queue and process retirement. A protected `door-window` service owns window IDs,
focus, configure epochs, resource registration and CPU presentation. Portable
`graphics-core` contains the state machine and renderer; `graphics-text` contains
bounded fixed-font parsing/rasterization. The public SDK uses the existing
authenticated service transport. Files and Notes are separate UID1001 processes
with real persistent Storage operations and no display authority.

Producer sections remain writable by the producer. The service never holds Rust
references to them: kernel assembly copies capture a bounded section lease into
validated private output. Premultiplication and layout are checked before replacing
a safe immutable snapshot. Release means capture completed; CPU presentation means
checked GOP writes completed. Neither operation implies vblank or GPU ownership.

Move reuses cached content and keeps the configure epoch. Resize replaces the
epoch and preserves clipped prior content until an acknowledged new buffer arrives.
Queued pointer positions are content-local at receipt time. Input saturation
revokes the consumer's scene and closes its channel logically; reconnect is explicit.
Service death withdraws the endpoint before a bounded restart; surviving clients
recreate windows and repaint from retained application data. Init may terminate a
known hung producer only on an authenticated request from the current server.

The initial output is 1024x768, 96 DPI, single-vCPU Q35/TCG, premultiplied BGRA8,
CPU compose/copy, with an emulated cursor plane. Resource and error limits are
specified in native-window-v1. A separate immutable test profile admits the
malicious producer and service-loss injection. Production boot has neither.

Microsoft Fluent SVGs are pinned MIT assets, rasterized by a host-only exporter.
Selawik is an OFL substitution. The newest public Windows release and UI changes
are recorded in the reference packet. P11 development pixels do not qualify P14
Windows shell fidelity; P12 controls/text and P13 User32/GDI remain distinct gates.
