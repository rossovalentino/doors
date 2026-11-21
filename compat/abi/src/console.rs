//! Canonical ASCII terminal lines survive small ReadFile requests.
pub struct Line {
    pub bytes: [u8; 1024],
    pub length: usize,
    pub position: usize,
    pub ready: bool,
}
impl Line {
    pub const fn new() -> Self {
        Self {
            bytes: [0; 1024],
            length: 0,
            position: 0,
            ready: false,
        }
    }
    pub fn push(&mut self, byte: u8) -> Result<(), u32> {
        if matches!(byte, 8 | 127) {
            self.length = self.length.saturating_sub(1);
            return Ok(());
        }
        if self.length >= self.bytes.len() - 1 && byte != b'\n' {
            return Err(8);
        }
        self.bytes[self.length] = byte;
        self.length += 1;
        self.ready = byte == b'\n';
        Ok(())
    }
    pub fn take(&mut self, out: &mut [u8]) -> usize {
        if !self.ready {
            return 0;
        }
        let n = out.len().min(self.length - self.position);
        out[..n].copy_from_slice(&self.bytes[self.position..self.position + n]);
        self.position += n;
        if self.position == self.length {
            self.bytes.fill(0);
            self.length = 0;
            self.position = 0;
            self.ready = false;
        }
        n
    }
}
impl Default for Line {
    fn default() -> Self {
        Self::new()
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn canonical_editing_survives_one_byte_reads() {
        let mut line = Line::new();
        for byte in b"print 124\x083\n" {
            line.push(*byte).unwrap();
        }
        let mut out = [0; 10];
        for byte in &mut out {
            assert_eq!(line.take(core::slice::from_mut(byte)), 1);
        }
        assert_eq!(&out, b"print 123\n");
        assert!(!line.ready);
        assert!(line.bytes.iter().all(|b| *b == 0));
        for _ in 0..1023 {
            line.push(b'x').unwrap();
        }
        assert_eq!(line.push(b'x'), Err(8));
        line.push(b'\n').unwrap();
        assert!(line.ready);
    }
}
