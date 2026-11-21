use door_pe_loader::x64_unwind::{
    Context, Error, Function, Memory, Pointers, UNW_FLAG_EHANDLER, UNW_FLAG_NHANDLER,
    UNW_FLAG_UHANDLER, virtual_unwind,
};

const BASE: u64 = 0x1_4000_0000;
const STACK: u64 = 0x7000_0000;
/// The stack pointer at function entry (the return address is here).
const S0: u64 = STACK + 0x80;
const RETURN: u64 = 0x1_4000_0abc;
const OLD_RBP: u64 = 0x1111;
const OLD_RBX: u64 = 0x2222;

struct Fixture {
    image: Vec<u8>,
    stack: Vec<u8>,
}
impl Memory for Fixture {
    fn read(&mut self, address: u64, output: &mut [u8]) -> Result<(), Error> {
        for (base, bytes) in [(BASE, &self.image), (STACK, &self.stack)] {
            if let Some(offset) = address.checked_sub(base).map(|v| v as usize)
                && let Some(source) = bytes.get(offset..offset + output.len())
            {
                output.copy_from_slice(source);
                return Ok(());
            }
        }
        Err(Error::Memory)
    }
}
impl Fixture {
    fn put(&mut self, rva: usize, bytes: &[u8]) {
        self.image[rva..rva + bytes.len()].copy_from_slice(bytes);
    }
    fn word(&mut self, address: u64, value: u64) {
        let at = (address - STACK) as usize;
        self.stack[at..at + 8].copy_from_slice(&value.to_le_bytes());
    }
}

/// push rbp; push rbx; sub rsp,0x20; lea rbp,[rsp+0x10]; body;
/// add rsp,0x20; pop rbx; pop rbp; ret.
const FUNCTION: Function = Function {
    begin: 0x100,
    end: 0x128,
    unwind: 0x300,
};
fn fixture(flags: u8) -> Fixture {
    let mut m = Fixture {
        image: vec![0x90; 0x1000],
        stack: vec![0; 0x100],
    };
    m.put(
        0x100,
        &[
            0x55, 0x53, 0x48, 0x83, 0xec, 0x20, 0x48, 0x8d, 0x6c, 0x24, 0x10,
        ],
    );
    m.put(0x120, &[0x48, 0x83, 0xc4, 0x20, 0x5b, 0x5d, 0xc3]);
    // UNWIND_INFO: version 1, flags, prolog 11, 4 codes, frame rbp at +16.
    m.put(0x300, &[1 | (flags << 3), 11, 4, 0x15]);
    m.put(0x304, &[11, 0x03, 6, 0x32, 2, 0x30, 1, 0x50]);
    // Handler RVA 0x700 and its data after the (even) code array.
    m.put(0x30c, &0x700u32.to_le_bytes());
    m.word(S0, RETURN);
    m.word(S0 - 8, OLD_RBP);
    m.word(S0 - 16, OLD_RBX);
    m
}
fn body_context() -> Context {
    let mut c = Context {
        registers: [0; 16],
        rip: BASE + 0x110,
        xmm: [[0; 16]; 16],
    };
    c.registers[4] = S0 - 48;
    c.registers[5] = S0 - 32;
    c.registers[3] = 0x9999;
    c
}

#[test]
fn a_body_frame_unwinds_through_the_frame_pointer_and_records_pointers() {
    let mut m = fixture(0);
    let mut c = body_context();
    let mut p = Pointers::default();
    let frame = virtual_unwind(
        &mut m,
        UNW_FLAG_NHANDLER,
        BASE,
        c.rip,
        FUNCTION,
        &mut c,
        &mut p,
    )
    .unwrap();
    assert_eq!(c.rip, RETURN);
    assert_eq!(c.registers[4], S0 + 8);
    assert_eq!(c.registers[5], OLD_RBP);
    assert_eq!(c.registers[3], OLD_RBX);
    assert_eq!(p.integer[3], S0 - 16);
    assert_eq!(p.integer[5], S0 - 8);
    assert_eq!(frame.establisher, S0 - 48);
    assert_eq!(frame.handler, None);
}

#[test]
fn a_partial_prolog_applies_only_the_codes_already_executed() {
    let mut m = fixture(0);
    let mut c = body_context();
    // After push rbp; push rbx, before sub rsp.
    c.rip = BASE + 0x102;
    c.registers[4] = S0 - 16;
    let mut p = Pointers::default();
    virtual_unwind(
        &mut m,
        UNW_FLAG_NHANDLER,
        BASE,
        c.rip,
        FUNCTION,
        &mut c,
        &mut p,
    )
    .unwrap();
    assert_eq!(c.rip, RETURN);
    assert_eq!(c.registers[4], S0 + 8);
    assert_eq!(c.registers[3], OLD_RBX);
}

#[test]
fn an_epilogue_is_simulated_from_any_instruction() {
    for (rva, rsp) in [
        (0x120u64, S0 - 48),
        (0x124, S0 - 16),
        (0x125, S0 - 8),
        (0x126, S0),
    ] {
        let mut m = fixture(0);
        let mut c = body_context();
        c.rip = BASE + rva;
        c.registers[4] = rsp;
        if rva >= 0x125 {
            c.registers[3] = OLD_RBX;
        }
        if rva == 0x126 {
            c.registers[5] = OLD_RBP;
        }
        let mut p = Pointers::default();
        let frame = virtual_unwind(
            &mut m,
            UNW_FLAG_EHANDLER,
            BASE,
            c.rip,
            FUNCTION,
            &mut c,
            &mut p,
        )
        .unwrap();
        assert_eq!(c.rip, RETURN, "rva {rva:#x}");
        assert_eq!(c.registers[4], S0 + 8, "rva {rva:#x}");
        assert_eq!(c.registers[3], OLD_RBX, "rva {rva:#x}");
        assert_eq!(c.registers[5], OLD_RBP, "rva {rva:#x}");
        // No handler runs for a frame that is already returning.
        assert_eq!(frame.handler, None);
    }
}

#[test]
fn handlers_come_back_only_past_the_prolog_and_for_the_kind_asked() {
    let mut m = fixture(1); // UNW_FLAG_EHANDLER
    let mut c = body_context();
    let mut p = Pointers::default();
    let frame = virtual_unwind(
        &mut m,
        UNW_FLAG_EHANDLER,
        BASE,
        c.rip,
        FUNCTION,
        &mut c,
        &mut p,
    )
    .unwrap();
    assert_eq!(frame.handler, Some((BASE + 0x700, BASE + 0x310)));
    let mut c = body_context();
    let frame = virtual_unwind(
        &mut m,
        UNW_FLAG_UHANDLER,
        BASE,
        c.rip,
        FUNCTION,
        &mut c,
        &mut p,
    )
    .unwrap();
    assert_eq!(frame.handler, None);
    let mut c = body_context();
    c.rip = BASE + 0x102;
    c.registers[4] = S0 - 16;
    let frame = virtual_unwind(
        &mut m,
        UNW_FLAG_EHANDLER,
        BASE,
        c.rip,
        FUNCTION,
        &mut c,
        &mut p,
    )
    .unwrap();
    assert_eq!(frame.handler, None);
}

#[test]
fn chained_info_continues_with_the_parent_function() {
    let mut m = fixture(0);
    // A fragment at 0x200..0x220 with no codes of its own, chained to the
    // parent function.
    m.put(0x400, &[1 | (4 << 3), 0, 0, 0]);
    m.put(0x404, &0x100u32.to_le_bytes());
    m.put(0x408, &0x128u32.to_le_bytes());
    m.put(0x40c, &0x300u32.to_le_bytes());
    let fragment = Function {
        begin: 0x200,
        end: 0x220,
        unwind: 0x400,
    };
    let mut c = body_context();
    c.rip = BASE + 0x210;
    let mut p = Pointers::default();
    virtual_unwind(
        &mut m,
        UNW_FLAG_NHANDLER,
        BASE,
        c.rip,
        fragment,
        &mut c,
        &mut p,
    )
    .unwrap();
    assert_eq!(c.rip, RETURN);
    assert_eq!(c.registers[4], S0 + 8);
    assert_eq!(c.registers[3], OLD_RBX);
}

#[test]
fn a_machine_frame_restores_rip_and_rsp_without_a_return_pop() {
    let mut m = fixture(0);
    // One code: PUSH_MACHFRAME (op 10, no error code) at offset 0.
    m.put(0x500, &[1, 0, 1, 0, 0, 0x0a, 0, 0]);
    let trap = Function {
        begin: 0x600,
        end: 0x610,
        unwind: 0x500,
    };
    let base_rsp = STACK + 0x20;
    m.word(base_rsp, 0x1_4000_0777); // RIP
    m.word(base_rsp + 24, STACK + 0xf0); // RSP
    let mut c = body_context();
    c.rip = BASE + 0x604;
    c.registers[4] = base_rsp;
    let mut p = Pointers::default();
    virtual_unwind(&mut m, UNW_FLAG_NHANDLER, BASE, c.rip, trap, &mut c, &mut p).unwrap();
    assert_eq!(c.rip, 0x1_4000_0777);
    assert_eq!(c.registers[4], STACK + 0xf0);
}

#[test]
fn a_tail_call_jump_ends_an_epilogue() {
    let mut m = fixture(0);
    // sub rsp,0x28 prolog; epilogue add rsp,0x28; jmp far outside.
    m.put(0x800, &[0x48, 0x83, 0xec, 0x28]);
    m.put(
        0x810,
        &[0x48, 0x83, 0xc4, 0x28, 0xe9, 0x00, 0x10, 0x00, 0x00],
    );
    m.put(0x900, &[1, 4, 1, 0, 4, 0x42]);
    let function = Function {
        begin: 0x800,
        end: 0x820,
        unwind: 0x900,
    };
    let mut c = body_context();
    c.rip = BASE + 0x810;
    c.registers[4] = S0 - 0x28;
    let mut p = Pointers::default();
    virtual_unwind(
        &mut m,
        UNW_FLAG_NHANDLER,
        BASE,
        c.rip,
        function,
        &mut c,
        &mut p,
    )
    .unwrap();
    assert_eq!(c.rip, RETURN);
    assert_eq!(c.registers[4], S0 + 8);
}

#[test]
fn malformed_info_and_unreadable_stacks_leave_the_context_unchanged() {
    let mut m = fixture(0);
    m.put(0x300, &[3, 11, 4, 0x15]); // version 3
    let mut c = body_context();
    let before = c;
    let mut p = Pointers::default();
    assert_eq!(
        virtual_unwind(
            &mut m,
            UNW_FLAG_NHANDLER,
            BASE,
            c.rip,
            FUNCTION,
            &mut c,
            &mut p
        ),
        Err(Error::Unsupported)
    );
    assert_eq!(c, before);
    let mut m = fixture(0);
    c.registers[5] = 0x10; // frame pointer into unreadable memory
    assert!(
        virtual_unwind(
            &mut m,
            UNW_FLAG_NHANDLER,
            BASE,
            c.rip,
            FUNCTION,
            &mut c,
            &mut p
        )
        .is_err()
    );
    assert_eq!(c.registers[5], 0x10);
}
