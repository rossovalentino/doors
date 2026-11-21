# 0010: Owned modern PCI storage transport

Status: accepted for P06 implementation; qualification belongs to the P06 report.

P06 needs real persistent block I/O without tying filesystem behavior to PCI
registers or surrendering memory ownership to unbounded hardware work. The
selected q35 profile therefore binds exactly one modern VirtIO block function
(`1af4:1042`). The existing transitional firmware disk is not selected.

The original source-versioned framework is divided into PCI discovery/resource
validation, modern VirtIO transport, split-queue ownership and the block class.
It does not establish a Rust binary driver ABI. Filesystems depend only on the
portable `storage::BlockDevice` contract. The modern transport owns all BAR
mapping tokens and one eight-page contiguous DMA token. The MM layer separately
rejects overlap with firmware RAM, runtime mappings and platform controllers.

The first implementation polls a single 16-descriptor split queue. Four slots
retain separate headers, status bytes and 4 KiB bounce buffers. Generations do
not wrap; cancellation before publication is distinct from an in-flight request.
Used indices, descriptor heads, completion lengths and status bytes are checked
before a slot can be reused. A write completion is not a durability promise;
the negotiated FLUSH operation establishes the transport's persistence boundary.

Reset first stops submissions. DMA backing remains owned until hardware reports
status zero and PCI bus mastering is confirmed disabled. Failed or timed-out
shutdown retains backing deliberately. Successful shutdown transfers every token
back to the memory owner for explicit unmapping and frame release. Reset writes
may already have changed storage; diagnostics use an explicitly reserved scratch
sector outside partitions and never assume cancellation rolled back a write.

The bounded profile deliberately excludes legacy transport, packed queues,
indirect descriptors, multi-queue, MSI/MSI-X dispatch, IOMMU translation and live
hotplug recovery. Device loss or malformed completion fails closed. Interrupt and
hotplug integration can extend the transport while preserving its ownership and
block interfaces; no placeholder service or class hierarchy is introduced here.

The normative reference is OASIS VirtIO 1.2 CS01 (1 July 2022), sections 2.1,
2.6, 3.1, 4.1 and 5.2. The acquired QEMU 10.2.0 PCI register/host definitions
confirm the selected emulator's configuration mechanism and BAR semantics.
Only interface facts informed the original implementation; third-party driver
source is neither incorporated nor linked. Full acquisition identities and
license classifications are recorded in the central source ledger.

The detailed resource, error, ordering and diagnostic contract is
[`storage-transport-v1.md`](../../specs/contracts/storage-transport-v1.md).
