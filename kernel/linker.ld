ENTRY(door_entry)
PHDRS { text PT_LOAD FLAGS(5); rodata PT_LOAD FLAGS(4); data PT_LOAD FLAGS(6); }
SECTIONS {
  . = 0xffffffff80000000;
  .text : ALIGN(4096) { KEEP(*(.text.boot)) *(.text .text.*) } :text
  . = ALIGN(4096);
  .rodata : { *(.rodata .rodata.*) *(.eh_frame_hdr) *(.eh_frame) } :rodata
  . = ALIGN(4096);
  .data : { *(.data .data.*) *(.got .got.*) } :data
  .bss (NOLOAD) : { *(.bss .bss.*) *(COMMON) } :data
  /DISCARD/ : { *(.comment) *(.note*) }
}
