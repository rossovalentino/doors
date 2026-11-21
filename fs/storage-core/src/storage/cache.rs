use super::{BLOCK_BYTES, BlockDevice, Error, Result};
struct Entry {
    block: u32,
    valid: bool,
    bytes: [u8; BLOCK_BYTES],
}
pub(super) struct Cache<D> {
    pub device: D,
    entry: Entry,
    pub poisoned: bool,
}
impl<D: BlockDevice> Cache<D> {
    pub fn new(device: D) -> Self {
        Self {
            device,
            entry: Entry {
                block: 0,
                valid: false,
                bytes: [0; BLOCK_BYTES],
            },
            poisoned: false,
        }
    }
    pub fn invalidate(&mut self) {
        self.entry.valid = false;
    }
    /// Borrow the sole cache page, avoiding a second page-sized stack buffer for
    /// read-only metadata decoding. The borrow prevents replacement while live.
    pub fn block(&mut self, block: u32) -> Result<&[u8; BLOCK_BYTES]> {
        if self.poisoned {
            return Err(Error::Poisoned);
        }
        if self.entry.valid && self.entry.block == block {
            return Ok(&self.entry.bytes);
        }
        // Prefix writes on a failed read must never leave the old tag valid.
        self.invalidate();
        if let Err(error) = self
            .device
            .read_sectors(u64::from(block) * 8, &mut self.entry.bytes)
        {
            self.invalidate();
            return Err(error);
        }
        self.entry.block = block;
        self.entry.valid = true;
        Ok(&self.entry.bytes)
    }
    pub fn read(&mut self, block: u32, out: &mut [u8; BLOCK_BYTES]) -> Result<()> {
        out.copy_from_slice(self.block(block)?);
        Ok(())
    }
    pub fn write(&mut self, block: u32, bytes: &[u8; BLOCK_BYTES]) -> Result<()> {
        if self.poisoned {
            return Err(Error::Poisoned);
        }
        if let Err(error) = self.device.write_sectors(u64::from(block) * 8, bytes) {
            self.poisoned = true;
            self.invalidate();
            return Err(error);
        }
        self.install(block, bytes);
        Ok(())
    }
    fn install(&mut self, block: u32, bytes: &[u8; BLOCK_BYTES]) {
        self.entry.bytes.copy_from_slice(bytes);
        self.entry.block = block;
        self.entry.valid = true;
    }
    pub fn flush(&mut self) -> Result<()> {
        if self.poisoned {
            return Err(Error::Poisoned);
        }
        if let Err(e) = self.device.flush() {
            self.poisoned = true;
            self.invalidate();
            return Err(e);
        }
        Ok(())
    }
}
