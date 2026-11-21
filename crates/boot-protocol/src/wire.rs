use core::ops::Range;

use crate::{Error, HEADER_BYTES, MAGIC, MAX_BLOB_BYTES, MAX_TABLES, REQUIRED, TABLE_BYTES};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Header {
    pub total_bytes: u64,
    pub table_count: u32,
    pub tables_offset: u64,
    pub self_phys: u64,
}

impl Header {
    pub fn decode(bytes: &[u8]) -> Result<Self, Error> {
        let bytes = bytes.get(..HEADER_BYTES).ok_or(Error::ShortRecord)?;
        if bytes[..8] != MAGIC {
            return Err(Error::BadMagic);
        }
        if read::<2>(bytes, 8)? != 1_u16.to_le_bytes() || read::<2>(bytes, 10)? != [0; 2] {
            return Err(Error::UnsupportedVersion);
        }
        if u32::from_le_bytes(read(bytes, 12)?) != HEADER_BYTES as u32
            || u32::from_le_bytes(read(bytes, 36)?) != TABLE_BYTES as u32
        {
            return Err(Error::InvalidRecordSize);
        }
        if read::<8>(bytes, 24)? != [0; 8] || read::<8>(bytes, 56)? != [0; 8] {
            return Err(Error::ReservedBits);
        }
        let header = Self {
            total_bytes: u64::from_le_bytes(read(bytes, 16)?),
            table_count: u32::from_le_bytes(read(bytes, 32)?),
            tables_offset: u64::from_le_bytes(read(bytes, 40)?),
            self_phys: u64::from_le_bytes(read(bytes, 48)?),
        };
        header.validate()?;
        Ok(header)
    }

    pub fn encode(self, output: &mut [u8]) -> Result<(), Error> {
        self.validate()?;
        let bytes = output.get_mut(..HEADER_BYTES).ok_or(Error::ShortRecord)?;
        bytes.fill(0);
        bytes[..8].copy_from_slice(&MAGIC);
        write(bytes, 8, 1_u16.to_le_bytes());
        write(bytes, 12, (HEADER_BYTES as u32).to_le_bytes());
        write(bytes, 16, self.total_bytes.to_le_bytes());
        write(bytes, 32, self.table_count.to_le_bytes());
        write(bytes, 36, (TABLE_BYTES as u32).to_le_bytes());
        write(bytes, 40, self.tables_offset.to_le_bytes());
        write(bytes, 48, self.self_phys.to_le_bytes());
        Ok(())
    }

    fn validate(self) -> Result<(), Error> {
        if self.total_bytes > MAX_BLOB_BYTES as u64 || self.table_count > MAX_TABLES {
            return Err(Error::LimitExceeded);
        }
        if self.total_bytes < HEADER_BYTES as u64 || self.tables_offset < HEADER_BYTES as u64 {
            return Err(Error::OutOfBounds);
        }
        if !self.tables_offset.is_multiple_of(8) || !self.self_phys.is_multiple_of(8) {
            return Err(Error::Misaligned);
        }
        bounded_range(
            self.tables_offset,
            u64::from(self.table_count) * TABLE_BYTES as u64,
            self.total_bytes,
        )?;
        self.self_phys
            .checked_add(self.total_bytes)
            .ok_or(Error::Overflow)?;
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TableDescriptor {
    pub kind: u32,
    pub revision: u16,
    pub flags: u16,
    pub offset: u64,
    pub length: u64,
    pub entry_bytes: u32,
}

impl TableDescriptor {
    pub fn decode(input: &[u8]) -> Result<Self, Error> {
        let bytes = input.get(..TABLE_BYTES).ok_or(Error::ShortRecord)?;
        if read::<4>(bytes, 28)? != [0; 4] {
            return Err(Error::ReservedBits);
        }
        Ok(Self {
            kind: u32::from_le_bytes(read(bytes, 0)?),
            revision: u16::from_le_bytes(read(bytes, 4)?),
            flags: u16::from_le_bytes(read(bytes, 6)?),
            offset: u64::from_le_bytes(read(bytes, 8)?),
            length: u64::from_le_bytes(read(bytes, 16)?),
            entry_bytes: u32::from_le_bytes(read(bytes, 24)?),
        })
    }

    /// Checks the descriptor and its payload extent against the enclosing blob.
    pub fn validate(self, total_bytes: u64) -> Result<(), Error> {
        if self.flags & !REQUIRED != 0 {
            return Err(Error::ReservedBits);
        }
        if self.length == 0 {
            return Err(Error::EmptyRange);
        }
        if self.entry_bytes == 0 || !self.length.is_multiple_of(u64::from(self.entry_bytes)) {
            return Err(Error::InvalidRecordSize);
        }
        if !self.offset.is_multiple_of(8) {
            return Err(Error::Misaligned);
        }
        bounded_range(self.offset, self.length, total_bytes)?;
        Ok(())
    }

    pub fn encode(self, output: &mut [u8], total_bytes: u64) -> Result<(), Error> {
        self.validate(total_bytes)?;
        let bytes = output.get_mut(..TABLE_BYTES).ok_or(Error::ShortRecord)?;
        bytes.fill(0);
        write(bytes, 0, self.kind.to_le_bytes());
        write(bytes, 4, self.revision.to_le_bytes());
        write(bytes, 6, self.flags.to_le_bytes());
        write(bytes, 8, self.offset.to_le_bytes());
        write(bytes, 16, self.length.to_le_bytes());
        write(bytes, 24, self.entry_bytes.to_le_bytes());
        Ok(())
    }
}

pub(crate) fn read<const N: usize>(bytes: &[u8], offset: usize) -> Result<[u8; N], Error> {
    bytes
        .get(offset..offset + N)
        .ok_or(Error::ShortRecord)?
        .try_into()
        .map_err(|_| Error::ShortRecord)
}

pub(crate) fn write<const N: usize>(bytes: &mut [u8], offset: usize, value: [u8; N]) {
    bytes[offset..offset + N].copy_from_slice(&value);
}

pub(crate) fn bounded_range(start: u64, length: u64, total: u64) -> Result<Range<usize>, Error> {
    let end = start.checked_add(length).ok_or(Error::Overflow)?;
    if end > total || end > MAX_BLOB_BYTES as u64 {
        return Err(Error::OutOfBounds);
    }
    Ok(start as usize..end as usize)
}

pub(crate) fn overlaps(a: &Range<usize>, b: &Range<usize>) -> bool {
    a.start < b.end && b.start < a.end
}
