//! Intel SDM 325384-081US, sections 6.14 and 8.7. These are CPU wire layouts,
//! deliberately separate from the code which loads privileged registers.

pub const CODE_SELECTOR: u16 = 8;
pub const DATA_SELECTOR: u16 = 16;
pub const TSS_SELECTOR: u16 = 24;
pub const USER_DATA_SELECTOR: u16 = 40 | 3;
pub const USER_CODE_SELECTOR: u16 = 48 | 3;
/// 32-bit user code (compatibility mode, L=0 D=1) for x86 programs (P17):
/// GDT entry 7. 0x2B data and 0x33 64-bit code are shared with x64 code.
pub const USER_CODE32_SELECTOR: u16 = 56 | 3;
/// The 32-bit code segment descriptor: base 0, limit 4 GB, DPL 3, D=1.
pub const USER_CODE32_DESCRIPTOR: u64 = 0x00cf_fa00_0000_ffff;
pub const USER_MIN: u64 = 0x1_0000;
pub const USER_END: u64 = 0x8000_0000_0000;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct Gate {
    offset_low: u16,
    selector: u16,
    ist: u8,
    attributes: u8,
    offset_middle: u16,
    offset_high: u32,
    reserved: u32,
}

impl Gate {
    pub const EMPTY: Self = Self::interrupt(0, 0);

    pub const fn interrupt(address: u64, ist: u8) -> Self {
        Self {
            offset_low: address as u16,
            selector: CODE_SELECTOR,
            ist,
            attributes: 0x8e,
            offset_middle: (address >> 16) as u16,
            offset_high: (address >> 32) as u32,
            reserved: 0,
        }
    }
    /// An interrupt gate user mode may invoke (DPL 3): `int3` raises a
    /// breakpoint rather than a general-protection fault.
    pub const fn user_interrupt(address: u64, ist: u8) -> Self {
        let mut gate = Self::interrupt(address, ist);
        gate.attributes = 0xee;
        gate
    }
}

#[repr(C, packed)]
pub struct DescriptorPointer {
    pub limit: u16,
    pub base: u64,
}

/// Fifteen software-saved GPRs, normalized vector/error, then the five values
/// always pushed by long-mode hardware, even for an interrupted ring-0 thread.
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct TrapFrame {
    pub r15: u64,
    pub r14: u64,
    pub r13: u64,
    pub r12: u64,
    pub r11: u64,
    pub r10: u64,
    pub r9: u64,
    pub r8: u64,
    pub rdi: u64,
    pub rsi: u64,
    pub rbp: u64,
    pub rdx: u64,
    pub rcx: u64,
    pub rbx: u64,
    pub rax: u64,
    pub vector: u64,
    pub error: u64,
    pub rip: u64,
    pub cs: u64,
    pub rflags: u64,
    pub rsp: u64,
    pub ss: u64,
}

/// Saved interrupt state below a separately aligned `TrapFrame`. Assembly moves
/// to this storage before committing ownership of the outgoing thread's stack.
#[repr(C, align(16))]
pub struct Context {
    pub fx: [u8; 512],
    pub frame: u64,
    pub fs_base: u64,
    pub cr3: u64,
    pub kernel_stack_top: u64,
    /// Restore the entry GS domain, including a kernel NMI in a SWAPGS window.
    pub restore_user_gs: u64,
    pub user_gs_base: u64,
}

/// Initial register contract selected by the trusted image loader. Native
/// startup and Windows bootstrap remain distinct from the saved GS domain.
#[derive(Clone, Copy)]
pub enum EntryArguments {
    Native { startup: u64, bytes: u64 },
    Windows { bootstrap: u64 },
}
impl EntryArguments {
    pub fn apply(self, frame: &mut TrapFrame) {
        match self {
            Self::Native { startup, bytes } => {
                frame.rdi = startup;
                frame.rsi = bytes;
            }
            Self::Windows { bootstrap } => frame.rcx = bootstrap,
        }
    }
}

impl Context {
    pub fn fresh(frame: u64, fs_base: u64) -> Self {
        let mut context = Self {
            fx: [0; 512],
            frame,
            fs_base,
            cr3: 0,
            kernel_stack_top: 0,
            restore_user_gs: 0,
            user_gs_base: 0,
        };
        context.fx[0..2].copy_from_slice(&0x037fu16.to_le_bytes());
        context.fx[24..28].copy_from_slice(&0x1f80u32.to_le_bytes());
        context
    }
}

/// Pure validation of state that an IRET to native user privilege may consume.
/// Mapping ownership and executable/readable backing are the process VM owner's
/// separate responsibility. A bad mapping faults at CPL3 and remains contained.
pub fn valid_user_return(context: &Context, frame: &TrapFrame, kernel_cr3: u64) -> bool {
    const FLAGS: u64 = 0x25_0fd7; // Arithmetic, TF/IF/DF, RF, AC and ID only.
    // 64-bit code, or 32-bit code (compatibility mode) whose instruction
    // and stack pointers are 32-bit addresses.
    let code = frame.cs == u64::from(USER_CODE_SELECTOR)
        || frame.cs == u64::from(USER_CODE32_SELECTOR)
            && frame.rip < 0x1_0000_0000
            && frame.rsp < 0x1_0000_0000;
    code && frame.ss == u64::from(USER_DATA_SELECTOR)
        && (USER_MIN..USER_END).contains(&frame.rip)
        && (USER_MIN..USER_END).contains(&frame.rsp)
        && frame.rflags & !FLAGS == 0
        && frame.rflags & 0x202 == 0x202
        && context.fs_base < USER_END
        && context.user_gs_base < USER_END
        && valid_cr3(context.cr3)
        && context.cr3 != kernel_cr3
        && context.kernel_stack_top >= 0xffff_8000_0000_1000
        && context.kernel_stack_top.is_multiple_of(16)
        && context.restore_user_gs == 1
}

pub const fn valid_cr3(cr3: u64) -> bool {
    cr3 != 0 && cr3 & !0x000f_ffff_ffff_f000 == 0
}

pub const fn tss_descriptor(base: u64) -> [u64; 2] {
    [
        103 | ((base & 0xff_ffff) << 16) | (0x89 << 40) | (((base >> 24) & 255) << 56),
        base >> 32,
    ]
}

pub fn tss_bytes(stack_tops: [u64; 4]) -> [u8; 104] {
    let mut bytes = [0; 104];
    // RSP0 is installed with the selected thread's trusted stack before CPL3.
    for (index, top) in stack_tops.into_iter().enumerate() {
        let start = 36 + index * 8;
        bytes[start..start + 8].copy_from_slice(&top.to_le_bytes());
    }
    // Outside the segment limit: all user I/O denied when CPL3 is introduced.
    bytes[102..104].copy_from_slice(&104u16.to_le_bytes());
    bytes
}

pub const fn pm_elapsed(start: u32, end: u32, bits: u8) -> u32 {
    end.wrapping_sub(start) & if bits == 24 { 0xff_ffff } else { u32::MAX }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::mem::{offset_of, size_of};

    #[test]
    fn architectural_layouts_match_hardware_and_entry_assembly() {
        assert_eq!(size_of::<Gate>(), 16);
        assert_eq!(size_of::<DescriptorPointer>(), 10);
        assert_eq!(size_of::<TrapFrame>(), 176);
        assert_eq!(offset_of!(TrapFrame, vector), 120);
        assert_eq!(offset_of!(TrapFrame, error), 128);
        assert_eq!(offset_of!(TrapFrame, rip), 136);
        assert_eq!(offset_of!(TrapFrame, rsp), 160);
        assert_eq!(size_of::<Context>(), 560);
        assert_eq!(offset_of!(Context, frame), 512);
        assert_eq!(offset_of!(Context, fs_base), 520);
        assert_eq!(offset_of!(Context, cr3), 528);
        assert_eq!(offset_of!(Context, kernel_stack_top), 536);
        assert_eq!(offset_of!(Context, restore_user_gs), 544);
        assert_eq!(offset_of!(Context, user_gs_base), 552);
        let gate = Gate::interrupt(0x1234_5678_9abc_def0, 2);
        assert_eq!(gate.offset_low, 0xdef0);
        assert_eq!(gate.offset_middle, 0x9abc);
        assert_eq!(gate.offset_high, 0x1234_5678);
        assert_eq!(gate.selector, 8);
        assert_eq!(gate.ist, 2);
        assert_eq!(gate.attributes, 0x8e);
        assert_eq!(gate.reserved, 0);
        let user = Gate::user_interrupt(0x1234_5678_9abc_def0, 1);
        assert_eq!(
            (user.attributes, user.ist, user.offset_low),
            (0xee, 1, 0xdef0)
        );
    }

    #[test]
    fn new_context_has_fresh_x87_and_simd_state() {
        let context = Context::fresh(0x1000, 0x2000);
        assert_eq!(context.frame, 0x1000);
        assert_eq!(context.fs_base, 0x2000);
        assert_eq!(&context.fx[..2], &[0x7f, 3]);
        assert_eq!(&context.fx[24..28], &[0x80, 0x1f, 0, 0]);
        assert_eq!(&context.fx[32..], &[0; 480]);
    }

    #[test]
    fn windows_entry_uses_rcx_and_preserves_separate_native_fs_and_user_gs() {
        let mut frame = TrapFrame::default();
        EntryArguments::Windows {
            bootstrap: 0x7100_4000,
        }
        .apply(&mut frame);
        assert_eq!(frame.rcx, 0x7100_4000);
        assert_eq!(
            (frame.rdi, frame.rsi, frame.rdx, frame.r8, frame.r9),
            (0, 0, 0, 0, 0)
        );
        let mut context = Context::fresh(0x1000, 0x7100_2000);
        context.user_gs_base = 0x7100_8000;
        assert_ne!(context.fs_base, context.user_gs_base);
        let mut native = TrapFrame::default();
        EntryArguments::Native {
            startup: 0x7100_0000,
            bytes: 1024,
        }
        .apply(&mut native);
        assert_eq!((native.rdi, native.rsi, native.rcx), (0x7100_0000, 1024, 0));
    }

    #[test]
    fn tss_encodes_full_addresses_and_disables_the_io_bitmap() {
        assert_eq!(
            tss_descriptor(0x1234_5678_9abc_def0),
            [0x9a00_89bc_def0_0067, 0x1234_5678]
        );
        let bytes = tss_bytes([0x1000, 0x2000, 0x3000, 0x4000]);
        assert_eq!(&bytes[36..44], &0x1000u64.to_le_bytes());
        assert_eq!(&bytes[60..68], &0x4000u64.to_le_bytes());
        assert_eq!(&bytes[..36], &[0; 36]);
        assert_eq!(&bytes[68..102], &[0; 34]);
        assert_eq!(&bytes[102..104], &[104, 0]);
    }

    #[test]
    fn timer_rollover_is_modulo_the_advertised_width() {
        assert_eq!(pm_elapsed(0xff_fff0, 0x10, 24), 32);
        assert_eq!(pm_elapsed(0xffff_fff0, 0x10, 32), 32);
        assert_eq!(pm_elapsed(500, 900, 24), 400);
    }

    #[test]
    fn user_return_rejects_privilege_and_noncanonical_state() {
        let mut context = Context::fresh(0xffff_8000_0000_1000, USER_MIN);
        context.cr3 = 0x2000;
        context.kernel_stack_top = 0xffff_8000_0001_0000;
        context.restore_user_gs = 1;
        let frame = TrapFrame {
            cs: u64::from(USER_CODE_SELECTOR),
            ss: u64::from(USER_DATA_SELECTOR),
            rip: USER_MIN,
            rsp: USER_END - 8,
            rflags: 0x202,
            ..TrapFrame::default()
        };
        assert!(valid_user_return(&context, &frame, 0x1000));
        for bit in [3, 5, 12, 13, 14, 15, 17, 19, 20, 22, 63] {
            let mut bad = frame;
            bad.rflags |= 1 << bit;
            assert!(!valid_user_return(&context, &bad, 0x1000));
        }
        for address in [0, USER_MIN - 1, USER_END, 0xffff_8000_0000_0000] {
            let mut bad = frame;
            bad.rip = address;
            assert!(!valid_user_return(&context, &bad, 0x1000));
            bad = frame;
            bad.rsp = address;
            assert!(!valid_user_return(&context, &bad, 0x1000));
        }
        let mut bad = frame;
        bad.cs = u64::from(CODE_SELECTOR);
        assert!(!valid_user_return(&context, &bad, 0x1000));
        // 32-bit code below 4 GB only.
        let mut x86 = frame;
        x86.cs = u64::from(USER_CODE32_SELECTOR);
        x86.rsp = 0xffff_fff0;
        assert!(valid_user_return(&context, &x86, 0x1000));
        let mut bad = x86;
        bad.rip = 0x1_0000_0000;
        assert!(!valid_user_return(&context, &bad, 0x1000));
        let mut bad = x86;
        bad.rsp = 0x1_0000_0008;
        assert!(!valid_user_return(&context, &bad, 0x1000));
        let mut bad = x86;
        bad.cs = 56;
        assert!(!valid_user_return(&context, &bad, 0x1000));
        context.cr3 = 0x1000;
        assert!(!valid_user_return(&context, &frame, 0x1000));
        assert!(!valid_cr3(0x2001));
        assert!(!valid_cr3(0));
    }
}
