use door_pe_loader::virtual_unwind::{Context, Error, Function, Memory, unwind};

const BASE: u64 = 0x140000000;
const STACK: u64 = 0x70000000;
struct Fixture {
    image: [u8; 1024],
    stack: [u8; 256],
}
impl Memory for Fixture {
    fn read(&mut self, address: u64, output: &mut [u8]) -> Result<(), Error> {
        for (base, bytes) in [
            (BASE, self.image.as_slice()),
            (STACK, self.stack.as_slice()),
        ] {
            if let Some(offset) = address
                .checked_sub(base)
                .and_then(|v| usize::try_from(v).ok())
                && let Some(source) = offset
                    .checked_add(output.len())
                    .and_then(|end| bytes.get(offset..end))
            {
                output.copy_from_slice(source);
                return Ok(());
            }
        }
        Err(Error::Memory)
    }
}
fn fixture(codes: &[u8], prolog: u8, frame: u8) -> (Fixture, Context, Function) {
    let mut memory = Fixture {
        image: [0x90; 1024],
        stack: [0; 256],
    };
    memory.image[512..516].copy_from_slice(&[1, prolog, (codes.len() / 2) as u8, frame]);
    memory.image[516..516 + codes.len()].copy_from_slice(codes);
    let mut context = Context {
        registers: [0; 16],
        rip: BASE + 128,
        xmm: [[0; 16]; 16],
    };
    context.registers[4] = STACK;
    (
        memory,
        context,
        Function {
            begin: 64,
            end: 256,
            unwind: 512,
        },
    )
}
fn run(
    memory: &mut Fixture,
    context: &mut Context,
    function: Function,
    pc: u64,
) -> Result<u64, Error> {
    unwind(memory, BASE, 1024, pc, function, context)
}
#[test]
fn body_and_partial_prolog_restore_only_executed_operations() {
    // sub rsp,32; push rbx: unwind in reverse instruction order.
    let (mut memory, initial, function) = fixture(&[5, 0x30, 4, 0x32], 5, 0);
    memory.stack[..8].copy_from_slice(&0x1234u64.to_le_bytes());
    memory.stack[40..48].copy_from_slice(&(BASE + 300).to_le_bytes());
    let mut context = initial;
    assert_eq!(
        run(&mut memory, &mut context, function, BASE + 128),
        Ok(STACK)
    );
    assert_eq!(context.registers[3], 0x1234);
    assert_eq!(context.registers[4], STACK + 48);
    assert_eq!(context.rip, BASE + 300);
    memory.stack[32..40].copy_from_slice(&(BASE + 320).to_le_bytes());
    context = initial;
    assert_eq!(
        run(&mut memory, &mut context, function, BASE + 68),
        Ok(STACK)
    );
    assert_eq!(context.registers[3], 0);
    assert_eq!(context.registers[4], STACK + 40);
    assert_eq!(context.rip, BASE + 320);
}
#[test]
fn frame_relative_saved_integer_and_simd_state_are_recovered() {
    let (mut memory, mut context, function) =
        fixture(&[8, 0x68, 4, 0, 7, 0x34, 6, 0, 4, 3, 2, 0x72], 8, 0x25);
    context.registers[5] = STACK + 32;
    memory.stack[48..56].copy_from_slice(&0xfeedu64.to_le_bytes());
    memory.stack[64..80].fill(0xa5);
    memory.stack[64..72].copy_from_slice(&(BASE + 320).to_le_bytes());
    let saved_simd: [u8; 16] = memory.stack[64..80].try_into().unwrap();
    assert_eq!(
        run(&mut memory, &mut context, function, BASE + 128),
        Ok(STACK)
    );
    assert_eq!(context.registers[3], 0xfeed);
    assert_eq!(context.xmm[6], saved_simd);
    assert_eq!(context.registers[4], STACK + 72);
}
#[test]
fn malformed_metadata_and_failed_stack_reads_are_transactional() {
    let (mut memory, initial, function) = fixture(&[5, 0x30, 4, 0x32], 5, 0);
    for header in [0, 9, 0x21] {
        memory.image[512] = header;
        let mut context = initial;
        assert_eq!(
            run(&mut memory, &mut context, function, BASE + 128),
            Err(Error::Unsupported)
        );
        assert_eq!(context, initial);
    }
    memory.image[512] = 1;
    let mut context = initial;
    context.registers[4] = STACK + 248;
    let saved = context;
    assert_eq!(
        run(&mut memory, &mut context, function, BASE + 128),
        Err(Error::Memory)
    );
    assert_eq!(context, saved);
    memory.image[518] = 6; // Code offsets must descend.
    context = initial;
    assert_eq!(
        run(&mut memory, &mut context, function, BASE + 128),
        Err(Error::Malformed)
    );
    assert_eq!(context, initial);
}
#[test]
fn terminal_return_does_not_apply_frame_allocation_twice() {
    let (mut memory, mut context, function) = fixture(&[4, 0x32], 4, 0);
    memory.image[128] = 0xc3;
    memory.stack[..8].copy_from_slice(&(BASE + 300).to_le_bytes());
    assert_eq!(
        run(&mut memory, &mut context, function, BASE + 128),
        Ok(STACK)
    );
    assert_eq!(context.registers[4], STACK + 8);
    assert_eq!(context.rip, BASE + 300);
    memory.image[128] = 0x5b;
    let saved = context;
    assert_eq!(
        run(&mut memory, &mut context, function, BASE + 128),
        Err(Error::Unsupported)
    );
    assert_eq!(context, saved);
}

#[test]
fn prefixed_returns_and_indirect_tail_jumps_fail_without_reapplying_prolog() {
    let (mut memory, initial, function) = fixture(&[4, 0x32], 4, 0);
    for encoding in [
        [0xf3, 0xc3, 0x90],
        [0x48, 0xc3, 0x90],
        [0xff, 0x20, 0x90],
        [0x48, 0xff, 0x20],
        [0x49, 0xff, 0xe0],
        [0x4f, 0x5b, 0x90],
        [0xf3, 0x48, 0xc3],
        [0x48, 0xf3, 0xc3],
        [0xf3, 0x48, 0xff],
        [0x66, 0xc3, 0x90],
    ] {
        memory.image[128..131].copy_from_slice(&encoding);
        let mut context = initial;
        assert_eq!(
            run(&mut memory, &mut context, function, BASE + 128),
            Err(Error::Unsupported)
        );
        assert_eq!(context, initial);
    }
}
