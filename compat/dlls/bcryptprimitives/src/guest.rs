//! bcryptprimitives.dll exports.
use crate::CHUNK;

#[global_allocator]
static HEAP: door_windows_abi::allocator::ProcessHeap = door_windows_abi::allocator::ProcessHeap;

#[link(name = "ntdll", kind = "dylib")]
unsafe extern "system" {
    fn DoorGenRandom(buffer: u64, length: u32) -> u32;
}

/// ProcessPrng(pbData, cbData): fills the buffer (FALSE only if the kernel
/// has no entropy to give, which Windows treats as fatal).
#[unsafe(no_mangle)]
pub extern "system" fn ProcessPrng(data: usize, bytes: usize) -> i32 {
    let mut done = 0usize;
    while done < bytes {
        let n = (bytes - done).min(CHUNK);
        if unsafe { DoorGenRandom((data + done) as u64, n as u32) } != 0 {
            return 0;
        }
        done += n;
    }
    1
}
