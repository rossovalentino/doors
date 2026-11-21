pub fn put16(b: &mut [u8], p: usize, v: u16) {
    b[p..p + 2].copy_from_slice(&v.to_le_bytes());
}
pub fn put32(b: &mut [u8], p: usize, v: u32) {
    b[p..p + 4].copy_from_slice(&v.to_le_bytes());
}
pub fn put64(b: &mut [u8], p: usize, v: u64) {
    b[p..p + 8].copy_from_slice(&v.to_le_bytes());
}
pub fn elf() -> Vec<u8> {
    let mut b = vec![0; 8192];
    b[..7].copy_from_slice(b"\x7fELF\x02\x01\x01");
    put16(&mut b, 16, 2);
    put16(&mut b, 18, 62);
    put32(&mut b, 20, 1);
    put64(&mut b, 24, 0xffff_ffff_8000_0000);
    put64(&mut b, 32, 64);
    put16(&mut b, 52, 64);
    put16(&mut b, 54, 56);
    put16(&mut b, 56, 1);
    put32(&mut b, 64, 1);
    put32(&mut b, 68, 5);
    put64(&mut b, 72, 4096);
    put64(&mut b, 80, 0xffff_ffff_8000_0000);
    put64(&mut b, 96, 512);
    put64(&mut b, 104, 4096);
    put64(&mut b, 112, 4096);
    b
}
pub fn pe() -> Vec<u8> {
    let mut b = vec![0; 1024];
    b[..2].copy_from_slice(b"MZ");
    put32(&mut b, 60, 64);
    b[64..68].copy_from_slice(b"PE\0\0");
    put16(&mut b, 68, 0x8664);
    put16(&mut b, 70, 1);
    put16(&mut b, 84, 240);
    put16(&mut b, 86, 2);
    put16(&mut b, 88, 0x20b);
    put32(&mut b, 104, 4096);
    put64(&mut b, 112, 0x140000000);
    put32(&mut b, 120, 4096);
    put32(&mut b, 124, 512);
    put32(&mut b, 144, 8192);
    put32(&mut b, 148, 512);
    put16(&mut b, 156, 10);
    put32(&mut b, 196, 16);
    put32(&mut b, 336, 512);
    put32(&mut b, 340, 4096);
    put32(&mut b, 344, 512);
    put32(&mut b, 348, 512);
    put32(&mut b, 364, 0x6000_0020);
    b
}
/// `pe()` as a PE32 (x86) image: machine 0x14c, optional header 0x10b with
/// a 4-byte image base and its data directories 16 bytes earlier.
#[allow(dead_code)]
pub fn pe32() -> Vec<u8> {
    let mut b = vec![0; 1024];
    b[..2].copy_from_slice(b"MZ");
    put32(&mut b, 60, 64);
    b[64..68].copy_from_slice(b"PE\0\0");
    put16(&mut b, 68, 0x14c);
    put16(&mut b, 70, 1);
    put16(&mut b, 84, 224);
    put16(&mut b, 86, 0x102);
    put16(&mut b, 88, 0x10b);
    put32(&mut b, 104, 4096);
    put32(&mut b, 116, 0x40_0000);
    put32(&mut b, 120, 4096);
    put32(&mut b, 124, 512);
    put32(&mut b, 144, 8192);
    put32(&mut b, 148, 512);
    put16(&mut b, 156, 3);
    put32(&mut b, 180, 16);
    put32(&mut b, 320, 512);
    put32(&mut b, 324, 4096);
    put32(&mut b, 328, 512);
    put32(&mut b, 332, 512);
    put32(&mut b, 348, 0x6000_0020);
    b
}
