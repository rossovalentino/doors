//! A little-endian buffer writer and a bounds-checked reader.
use alloc::vec::Vec;

/// Input that does not decode (short, oversized or out of order).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Malformed;

#[derive(Default, Clone, Debug)]
pub struct Writer {
    pub bytes: Vec<u8>,
}
impl Writer {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn u8(&mut self, v: u8) -> &mut Self {
        self.bytes.push(v);
        self
    }
    pub fn u16(&mut self, v: u16) -> &mut Self {
        self.bytes.extend_from_slice(&v.to_le_bytes());
        self
    }
    pub fn u32(&mut self, v: u32) -> &mut Self {
        self.bytes.extend_from_slice(&v.to_le_bytes());
        self
    }
    pub fn u64(&mut self, v: u64) -> &mut Self {
        self.bytes.extend_from_slice(&v.to_le_bytes());
        self
    }
    pub fn raw(&mut self, b: &[u8]) -> &mut Self {
        self.bytes.extend_from_slice(b);
        self
    }
    /// A length-prefixed byte string.
    pub fn blob(&mut self, b: &[u8]) -> &mut Self {
        self.u32(b.len() as u32).raw(b)
    }
    /// A BSTR: present flag (NULL differs from empty), length, UTF-16 units.
    pub fn bstr(&mut self, units: Option<&[u16]>) -> &mut Self {
        match units {
            None => self.u8(0),
            Some(u) => {
                self.u8(1).u32(u.len() as u32);
                for unit in u {
                    self.u16(*unit);
                }
                self
            }
        }
    }
    pub fn len(&self) -> usize {
        self.bytes.len()
    }
    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }
}

/// Reading past the end, or a length that does not fit, is `Err(Malformed)`.
pub struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
}
impl<'a> Reader<'a> {
    pub fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, at: 0 }
    }
    pub fn position(&self) -> usize {
        self.at
    }
    pub fn rest(&self) -> &'a [u8] {
        &self.bytes[self.at..]
    }
    pub fn raw(&mut self, n: usize) -> Result<&'a [u8], Malformed> {
        let end = self.at.checked_add(n).ok_or(Malformed)?;
        let b = self.bytes.get(self.at..end).ok_or(Malformed)?;
        self.at = end;
        Ok(b)
    }
    pub fn u8(&mut self) -> Result<u8, Malformed> {
        Ok(self.raw(1)?[0])
    }
    pub fn u16(&mut self) -> Result<u16, Malformed> {
        let b = self.raw(2)?;
        Ok(u16::from_le_bytes([b[0], b[1]]))
    }
    pub fn u32(&mut self) -> Result<u32, Malformed> {
        let b = self.raw(4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }
    pub fn u64(&mut self) -> Result<u64, Malformed> {
        let mut a = [0u8; 8];
        a.copy_from_slice(self.raw(8)?);
        Ok(u64::from_le_bytes(a))
    }
    pub fn guid(&mut self) -> Result<crate::Guid, Malformed> {
        let mut g = [0u8; 16];
        g.copy_from_slice(self.raw(16)?);
        Ok(g)
    }
    pub fn blob(&mut self) -> Result<&'a [u8], Malformed> {
        let n = self.u32()? as usize;
        self.raw(n)
    }
    pub fn bstr(&mut self) -> Result<Option<Vec<u16>>, Malformed> {
        if self.u8()? == 0 {
            return Ok(None);
        }
        let n = self.u32()? as usize;
        let b = self.raw(n.checked_mul(2).ok_or(Malformed)?)?;
        Ok(Some(
            b.chunks_exact(2)
                .map(|c| u16::from_le_bytes([c[0], c[1]]))
                .collect(),
        ))
    }
    pub fn done(&self) -> bool {
        self.at == self.bytes.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn values_round_trip_and_short_input_fails() {
        let mut w = Writer::new();
        w.u8(7).u16(0x1234).u32(0xdead_beef).u64(u64::MAX - 1);
        w.blob(b"abc")
            .bstr(None)
            .bstr(Some(&[0x44, 0x6f]))
            .bstr(Some(&[]));
        let mut r = Reader::new(&w.bytes);
        assert_eq!(r.u8(), Ok(7));
        assert_eq!(r.u16(), Ok(0x1234));
        assert_eq!(r.u32(), Ok(0xdead_beef));
        assert_eq!(r.u64(), Ok(u64::MAX - 1));
        assert_eq!(r.blob(), Ok(&b"abc"[..]));
        assert_eq!(r.bstr(), Ok(None));
        assert_eq!(r.bstr(), Ok(Some(alloc::vec![0x44, 0x6f])));
        assert_eq!(r.bstr(), Ok(Some(alloc::vec![])));
        assert!(r.done());
        assert_eq!(r.u8(), Err(Malformed));
        let mut short = Reader::new(&[5, 0, 0, 0, 1]);
        assert_eq!(short.blob(), Err(Malformed));
    }
}
