//! SDK19041 AMD64 exception/control-integer context bytes, separate from native
//! trap frames. Floating/debug/XSTATE mutation is outside this initial profile.
pub const CONTEXT_BYTES: usize = 1232;
pub const RECORD_BYTES: usize = 152;
pub const CONTEXT_OFFSET: usize = 160;
pub const PACKET_BYTES: usize = CONTEXT_OFFSET + CONTEXT_BYTES;
pub const CONTEXT_FLAGS: u32 = 0x0010_0003;
/// CONTEXT_FLOATING_POINT: FltSave (the FXSAVE image at 256) and MxCsr are
/// valid (P18: delivered faults carry them, so user mode can resume any
/// context with its floating-point state).
pub const CONTEXT_FLOATING_POINT: u32 = 0x0010_0008;
/// The FXSAVE image inside a CONTEXT.
pub const FLOAT_OFFSET: usize = 256;
pub const DISPATCH_EXPORT: &str = "DoorExceptionDispatch";
pub const ACCESS_VIOLATION: u32 = 0xc000_0005;
/// STATUS_GUARD_PAGE_VIOLATION: the first touch of a PAGE_GUARD page (the
/// guard is gone afterwards). Parameters as for an access violation.
pub const GUARD_PAGE_VIOLATION: u32 = 0x8000_0001;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct IntegerContext {
    /// RAX,RCX,RDX,RBX,RSP,RBP,RSI,RDI,R8..R15,RIP in SDK order.
    pub registers: [u64; 17],
    pub flags: u32,
}
impl IntegerContext {
    pub fn encode(self, output: &mut [u8]) -> bool {
        if output.len() != CONTEXT_BYTES {
            return false;
        }
        output.fill(0);
        output[48..52].copy_from_slice(&CONTEXT_FLAGS.to_le_bytes());
        output[56..58].copy_from_slice(&0x33u16.to_le_bytes());
        output[66..68].copy_from_slice(&0x2bu16.to_le_bytes());
        output[68..72].copy_from_slice(&self.flags.to_le_bytes());
        for (index, register) in self.registers.iter().enumerate() {
            output[120 + index * 8..128 + index * 8].copy_from_slice(&register.to_le_bytes());
        }
        true
    }
    pub fn decode(input: &[u8]) -> Option<Self> {
        if input.len() != CONTEXT_BYTES
            || u32::from_le_bytes(input[48..52].try_into().ok()?) & !0x8 != CONTEXT_FLAGS
            || u16::from_le_bytes(input[56..58].try_into().ok()?) != 0x33
            || u16::from_le_bytes(input[66..68].try_into().ok()?) != 0x2b
        {
            return None;
        }
        let value = Self {
            registers: core::array::from_fn(|index| {
                u64::from_le_bytes(
                    input[120 + index * 8..128 + index * 8]
                        .try_into()
                        .expect("checked context length"),
                )
            }),
            flags: u32::from_le_bytes(input[68..72].try_into().ok()?),
        };
        if value.flags & !0x25_0fd7 != 0
            || value.flags & 0x202 != 0x202
            || !(0x10000..0x8000_0000_0000).contains(&value.registers[4])
            || !(0x10000..0x8000_0000_0000).contains(&value.registers[16])
        {
            return None;
        }
        Some(value)
    }
}
pub fn record(output: &mut [u8], code: u32, instruction: u64, parameters: &[u64]) -> bool {
    if output.len() != RECORD_BYTES || parameters.len() > 15 {
        return false;
    }
    output.fill(0);
    output[..4].copy_from_slice(&code.to_le_bytes());
    output[16..24].copy_from_slice(&instruction.to_le_bytes());
    output[24..28].copy_from_slice(&(parameters.len() as u32).to_le_bytes());
    for (index, parameter) in parameters.iter().enumerate() {
        output[32 + index * 8..40 + index * 8].copy_from_slice(&parameter.to_le_bytes());
    }
    true
}

/// Put an FXSAVE image into a CONTEXT (FltSave and MxCsr) and mark it
/// CONTEXT_FLOATING_POINT.
pub fn encode_float(fx: &[u8; 512], context: &mut [u8]) -> bool {
    if context.len() != CONTEXT_BYTES {
        return false;
    }
    context[FLOAT_OFFSET..FLOAT_OFFSET + 512].copy_from_slice(fx);
    context[52..56].copy_from_slice(&fx[24..28]);
    let flags = u32::from_le_bytes(context[48..52].try_into().expect("context length"));
    context[48..52].copy_from_slice(&(flags | CONTEXT_FLOATING_POINT).to_le_bytes());
    true
}
/// The FXSAVE image of a CONTEXT that carries one with a valid MXCSR
/// (reserved bits clear, so FXRSTOR cannot fault on it).
pub fn decode_float(context: &[u8]) -> Option<[u8; 512]> {
    if context.len() != CONTEXT_BYTES {
        return None;
    }
    let flags = u32::from_le_bytes(context[48..52].try_into().ok()?);
    if flags & CONTEXT_FLOATING_POINT != CONTEXT_FLOATING_POINT {
        return None;
    }
    let fx: [u8; 512] = context[FLOAT_OFFSET..FLOAT_OFFSET + 512].try_into().ok()?;
    let mxcsr = u32::from_le_bytes(fx[24..28].try_into().ok()?);
    (mxcsr & 0xffff_0000 == 0).then_some(fx)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn integer_context_rejects_privileged_returns_and_unsupported_state() {
        let mut context = IntegerContext {
            registers: [0; 17],
            flags: 0x202,
        };
        context.registers[4] = 0x70001000;
        context.registers[16] = 0x140001000;
        let mut bytes = [0; CONTEXT_BYTES];
        assert!(context.encode(&mut bytes));
        assert_eq!(IntegerContext::decode(&bytes), Some(context));
        // CONTEXT_CONTROL missing, a wrong CS or SS: refused. (Bit 3 of the
        // flags is CONTEXT_FLOATING_POINT, which a context may carry.)
        for (offset, bit) in [(48, 1), (56, 8), (66, 8)] {
            let mut bad = bytes;
            bad[offset] ^= bit;
            assert_eq!(IntegerContext::decode(&bad), None);
        }
        let mut with_float = bytes;
        let mut fx = [0u8; 512];
        fx[24..28].copy_from_slice(&0x1f80u32.to_le_bytes());
        assert!(encode_float(&fx, &mut with_float));
        assert_eq!(IntegerContext::decode(&with_float), Some(context));
        assert_eq!(decode_float(&with_float), Some(fx));
        assert_eq!(decode_float(&bytes), None);
        // An MXCSR with reserved bits would fault FXRSTOR: not taken.
        with_float[FLOAT_OFFSET + 26] = 1;
        assert_eq!(decode_float(&with_float), None);
        for flags in [0, 2, 0x3202, 0x4202, 0x40202 | 0x80000000] {
            context.flags = flags;
            context.encode(&mut bytes);
            assert_eq!(IntegerContext::decode(&bytes), None);
        }
        context.flags = 0x202;
        for register in [4, 16] {
            for address in [0, 0xffff_8000_0000_0000, u64::MAX] {
                let old = context.registers[register];
                context.registers[register] = address;
                context.encode(&mut bytes);
                assert_eq!(IntegerContext::decode(&bytes), None);
                context.registers[register] = old;
            }
        }
    }
    #[test]
    fn exception_record_uses_sdk_extents_and_zero_padding() {
        let mut bytes = [0xff; RECORD_BYTES];
        assert!(record(
            &mut bytes,
            ACCESS_VIOLATION,
            0x140001000,
            &[1, 0x50000000]
        ));
        assert_eq!(&bytes[24..28], &2u32.to_le_bytes());
        assert_eq!(&bytes[40..48], &0x50000000u64.to_le_bytes());
        assert!(bytes[48..].iter().all(|byte| *byte == 0));
        assert!(!record(&mut bytes, 0, 0, &[0; 16]));
        assert_eq!(IntegerContext::decode(&[0; CONTEXT_BYTES - 1]), None);
    }
}
