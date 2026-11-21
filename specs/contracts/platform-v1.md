# Door platform summary v1

P03 adds BootInfo table kind `5`, revision `1`, one required 1024-byte record.
This is an original Door wire format; all multibyte fields are little endian.
The portable reader never dereferences a physical address. P02 memory-only and
boot validation remain supported; P03 requires `BootInfo::validate_platform()`.

| Offset | Encoding | Meaning |
|---|---|---|
| 0 | four u64 | Physical RSDP, XSDT, MADT, FADT addresses, in that order |
| 32 | four u32 | Full validated byte lengths, same order |
| 48 | u64 | Local APIC register page physical address |
| 56 | u32 | Bit 0 MADT PCAT compatibility, bit 1 FADT legacy devices; other bits zero |
| 60 | u16 | Enabled processor count, 1–64 |
| 62 | u16 | I/O APIC count, 1–8 |
| 64 | u16 | PM timer system-I/O base, or zero if unsupported |
| 66 | u8 | PM timer width: 24 or 32, or zero when absent |
| 67 | 13 bytes | Zero |
| 80 | 64 × 8 bytes | Processor: ACPI UID u32, APIC ID u32 |
| 592 | 8 × 16 bytes | I/O APIC: ID u32, GSI base u32, physical page address u64 |
| 720 | 304 bytes | Zero |

Unused array slots are zero. Processor UIDs and APIC IDs are unique; the xAPIC
bootstrap profile accepts enabled APIC IDs 0–254. CPU records describe firmware
availability, not online state or a BSP designation. The kernel checks its own
CPUID/APIC identity against the array and owns online accounting. I/O APIC IDs,
addresses and starting GSIs must be unique; hardware register reads establish
redirection-table size before routing. Local/I/O APIC addresses are distinct,
nonzero, 4 KiB aligned and wholly below 4 GiB. The kernel maps them as device
memory after establishing its own paging ownership.

The loader requires exactly one ACPI 2+ configuration-table GUID entry and a valid
ACPI 2+ RSDP/XSDT. There is no ACPI 1.0 GUID, RSDT or BIOS-memory-search fallback
in the selected x64 UEFI profile. Missing, duplicated or invalid required roots
fail loader initialization. Each source read first proves its entire extent is
inside a current firmware memory-map descriptor of type ACPI reclaim or ACPI NVS.
Table addresses must be nonzero and below 4 GiB. The adapter reads the fixed header,
checks length, then validates the complete extent before forming the full slice.
It makes no intervening firmware call or allocation while table views are borrowed.
The memory-map owner drops before the final ExitBootServices acquisition.

RSDP length is 36–4096 bytes; both legacy and full checksums are checked. Generic
ACPI tables are 36–65536 bytes, have a matching signature and full checksum. XSDT
revision is 1 with 1–128 whole, unique, nonzero 64-bit entries. Every referenced
table is range and checksum checked, including unused signatures. Exactly one MADT
and one FADT must exist. The four retained root extents may not overlap.

MADT records must have complete bounded lengths; type 0 local APIC, type 1 I/O
APIC, type 5 local APIC override and type 9 local x2APIC are parsed. Enabled and
online-capable processor identities must be unique within a 64-record bound;
only enabled CPUs are copied. ACPI declares a processor with both flags clear
unusable, so its identity is ignored. Type 9 IDs above the selected xAPIC range
are rejected when enabled. At most one local APIC override is accepted. Unknown
and unused bounded records are skipped. Interrupt-source overrides and NMI/AML
descriptions are not consumed: P03 masks external routing and does not infer ISA
interrupt wiring from this summary.

FADT revision 3 or later is required, with minimum complete lengths 244 bytes for
revisions 3/4, 268 for revision 5 and 276 for revision 6 or later. A usable nonzero
system-I/O X_PM_TMR GAS takes precedence over the legacy PM_TMR port. This profile
accepts GAS width 24/32, offset zero and unspecified/dword access size. An unusable
extended GAS falls back to the legacy port as allowed by the FADT contract.
Port range must fit a dword in 16-bit I/O space and PM_TMR_LEN must be 4. Width comes
from TMR_VAL_EXT. Hardware-reduced ACPI reports no fixed PM timer. Timer consumers
must reject absence or select a separately qualified source. LEGACY_DEVICES is
copied only as its specified advisory bit; it does not establish PIT presence.

The owned record contains copied facts plus provenance addresses; there are no
borrowed firmware pointers. The final BootInfo validator requires every retained
root extent to remain within ACPI reclaim/NVS memory with permanent/runtime
reservation, and rejects controller pages overlapping firmware RAM descriptors.
All ACPI reclaim/NVS backing stays reserved in P03, including unused tables and
the FADT's indirect AML/FACS references. Root tables need no kernel mapping because
the kernel reads the copied summary. Reclaim, AML, firmware runtime invocation,
hotplug and general power management require later ownership contracts.

Authoritative acquisition: ACPI Specification 6.5, sections 5.2.5 (RSDP), 5.2.6
(description header), 5.2.8 (XSDT), 5.2.9 (FADT), 5.2.12 (MADT), 4.8.2.1 and
4.8.3.3 (PM timer). UEFI Forum's official read-only source release commit
`5dabe6f37fbe753d90292a2a419105be5d55a778` is titled `Release ACPI_6.5`; its index
identifies ACPI Specification 6.5. Source files:

- [ACPI software programming model](https://github.com/UEFI/ACPI-Specification-Release/blob/5dabe6f37fbe753d90292a2a419105be5d55a778/source/05_ACPI_Software_Programming_Model.rst)
- [ACPI hardware specification](https://github.com/UEFI/ACPI-Specification-Release/blob/5dabe6f37fbe753d90292a2a419105be5d55a778/source/04_ACPI_Hardware_Specification.rst)

Validation includes truncated prefixes, checksums, duplicate/unknown/malformed
MADT entries, xAPIC limits, RSDP/XSDT bounds, FADT precedence/width/reduced hardware,
wire padding/counts/overlap and final firmware reservations. Actual firmware
acquisition and timer behavior require the P03 guest gate in addition to host tests.

## Revision 2: copied device presentation metadata

The current loader emits kind5 revision2. Offset67 is the OEM FADT
Preferred_PM_Profile (0 unspecified, 1..8 ACPI-defined); offsets68..79 stay zero.
Reserved OEM values above8 map to unspecified during acquisition. Revision1 still
requires67 zero and remains readable. Both revisions use1024bytes and identical
address/processor validation. The public HARDWARE_INFO syscall copies only the
profile and firmware processor count; it exposes no firmware addresses or grants.
This is advisory firmware classification, not SMBIOS chassis enumeration or a
physical-device driver claim. Source: ACPI6.6 section5.2.9, FADT offset45,
https://uefi.org/specs/ACPI/6.6/05_ACPI_Software_Programming_Model.html#fixed-acpi-description-table-fadt
