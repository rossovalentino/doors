# Storage transport v1

This is a source-level kernel interface for the P06 q35/modern VirtIO profile,
not a stable binary driver ABI. The implementation lives in `kernel/src/drivers`.

## Discovery and ownership

`unsafe pci::discover_block()` requires CPL0, PCI configuration mechanism 1 and
exclusive ownership of unbound block functions. It enumerates bounded BDF space,
selects exactly one `1af4:1042` function and rejects ambiguous identity. Individual
CF8/CFC transactions are serialized with interrupts locally masked. Header and
capability lists are bounded and cycle checked. Supported common, notification,
ISR and device capabilities must have valid lengths, alignment and nonoverlapping
ranges inside their sized memory BARs. Unsupported capability kinds are ignored.

Referenced firmware BARs are sized while I/O decoding, memory decoding and bus
mastering are disabled; their original assignments are restored. Supported BARs
are 32/64-bit memory apertures, power-of-two sized, 4 KiB through 16 MiB, aligned
to their size and below the 52-bit physical-address limit. Pairwise overlap and
capabilities addressing a 64-bit BAR's upper half are rejected. `Function::bars()`
returns six optional page-aligned `{ physical, bytes }` specifications. Before
mapping, the caller must use the MM resource validator against BootInfo memory,
existing mappings and platform controllers. Resources not owned by this function
must not be aliased. MSI, MSI-X and INTx are disabled for polling.

`unsafe VirtioBlock::initialize(function, bars, dma, clock)` consumes the function,
the six optional uncached BAR mappings and an eight-page contiguous, coherent,
exclusive RAM mapping with writable/NX permissions. The unsafe boundary requires
the mappings to correspond to the exact physical BAR specifications. The supplied
clock returns monotonic milliseconds. Every device wait also has a ten-million
iteration bound, so a stopped clock cannot make completion or reset wait forever.
The time deadline is two seconds; it is a failure bound, not a latency guarantee.

The current integration reserves virtual BAR windows at
`0xffffffffb1000000 + BAR_index * 0x01000000` and DMA at `0xffffffffb8000000`.
These addresses are policy supplied by the caller, not device addresses. DMA
descriptors always contain physical addresses from the retained RAM token.

## Initialization and data plane

The transport follows reset, ACKNOWLEDGE, DRIVER, feature selection, FEATURES_OK,
queue setup and DRIVER_OK ordering. It requires and negotiates exactly
`VIRTIO_F_VERSION_1` and `VIRTIO_BLK_F_FLUSH` (`0x0000000100000200`), rejects a
read-only device for this writable profile, and verifies feature acceptance.
No packed, indirect, event-index, access-platform or notification-data features
are negotiated. The driver reads capacity with the configuration generation
check and advertises 512-byte sectors through `storage::Geometry`.

The 16-entry split queue supports four concurrent owned slots, each using up to
three descriptors. Page zero contains descriptor/available/used rings and four
request header/status records; pages one through four are private 4 KiB bounce
buffers. Remaining pages belong to the same allocation and are retained until
shutdown. Headers and registers are little endian. MMIO 64-bit fields use ordered
low/high 32-bit accesses. DMA publication uses a release barrier before the
available index and a full barrier before the notification write; completion uses
an acquire barrier after the used index. Shared DMA fields use volatile accesses.

`BlockDevice` reads and writes reject empty, misaligned, overflowing or
out-of-capacity ranges before descriptor publication. Larger aligned requests
are split into at most 4 KiB hardware operations. A success covers the entire
requested slice; an error may leave an earlier prefix transferred. Write success
is initially volatile. `flush()` sends a real `VIRTIO_BLK_T_FLUSH` with sector zero
and no data descriptor. Persistence is limited by the configured backend's flush
contract; host-crash or power-failure testing is not implied.

## Completion, cancellation and removal

Slots progress from free to pending to published to terminal and back to free
only on reap. Each reservation has a fresh nonwrapping generation. A pending
request can be canceled; a published request cannot be represented as undone.
Device completions must reference a published slot's descriptor head, fit within
the number of published requests and use a known status. Unknown heads, duplicate
completion, overlong/zero used length and invalid status are malformed. A success
with fewer bytes than required is `ShortIo`; reads require data bytes plus status,
while writes and flushes require their status byte. Hardware I/O and unsupported
status are reported explicitly, with descriptors still retained until reap.

Timeout, device failure or malformed completion stops submissions and attempts
reset. Only observed status zero acknowledges reset and allows outstanding slots
to become reset terminals. PCI bus mastering is disabled and read back. Without
that acknowledgement, DMA mappings and frames are retained; they are never
returned to the allocator. A best-effort master disable also runs on timeout.
The normal public data plane remains stopped after such failure.

`shutdown(self)` returns `Resources { bars, dma }` only after acknowledged reset,
confirmed master disable and disabled address decoding. The caller then retires
MM mappings synchronously and frees the exact original eight-page frame token.
Dropped tokens intentionally retain backing under the MM contract. Surprise
removal, hotplug re-enumeration and automatic recovery are not supported claims.

## Evidence and limits

`selftest(scratch_lba)` fills the real pending queue, verifies queue-full and
prepublication cancellation, publishes a scratch write, acknowledges hardware
reset, rejects an obsolete token after restart and confirms unchanged capacity.
The caller dedicates the scratch sector outside every partition; its final
contents are indeterminate. Statistics count successful hardware read/write/flush
operations and resets. Unit tests additionally exercise malformed heads/status/
length, duplicate completion, index wrap, descriptor reuse and removal ownership.
These host tests do not establish actual hardware execution. The phase guest
report separately records real reads/writes/flushes, filesystem persistence,
injected short-I/O propagation, queue/reset diagnostics and DMA reclamation.

Primary references: [VirtIO 1.2 CS01](https://docs.oasis-open.org/virtio/virtio/v1.2/cs01/virtio-v1.2-cs01.html)
sections 2.1, 2.6, 3.1, 4.1.4–4.1.5 and 5.2;
[QEMU 10.2 PCI register interface](https://raw.githubusercontent.com/qemu/qemu/v10.2.0/include/standard-headers/linux/pci_regs.h),
[PCI host configuration interface](https://raw.githubusercontent.com/qemu/qemu/v10.2.0/hw/pci/pci_host.c),
and [q35 register placement](https://raw.githubusercontent.com/qemu/qemu/v10.2.0/hw/pci-host/q35.c).
