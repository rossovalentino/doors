//! Registry hive (`super::super::HIVE_NAME`): the key and value records past
//! the catalog's fixed slots, in the catalog's record layout.
//!
//! ```text
//!  0  "DOORREG1"   8  version (1)   12 key records   16 value records
//!  20 CRC-32 of the payload (bytes 64..)              24..64 zero
//!  64 key records (128 bytes each), then value records (192 bytes each)
//! ```
//!
//! Record j holds slot `CATALOG_KEYS + j` (`CATALOG_VALUES + j`). Only the
//! used extent is stored: the last used slot ends it, so a shorter hive
//! written over a longer one never resurrects the tail.
use super::*;

pub(crate) const HEADER: usize = 64;
const VERSION: u32 = 1;
/// Overflow slots in use: (key records, value records).
pub(crate) fn extent(state: &State) -> (usize, usize) {
    let keys = state.config.keys[CATALOG_KEYS..]
        .iter()
        .rposition(|k| k.id != 0)
        .map_or(0, |i| i + 1);
    let values = state.config.values[CATALOG_VALUES..]
        .iter()
        .rposition(|e| e.key != 0)
        .map_or(0, |i| i + 1);
    (keys, values)
}
pub(crate) fn bytes((keys, values): (usize, usize)) -> usize {
    HEADER + keys * KEY_RECORD + values * VALUE_RECORD
}
pub(crate) const CRC_START: u32 = !0;
fn crc_step(mut crc: u32, bytes: &[u8]) -> u32 {
    for &byte in bytes {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xedb88320u32.wrapping_mul(crc & 1));
        }
    }
    crc
}
/// Visit the payload as `scratch`-sized, record-aligned chunks:
/// `visit(payload offset, chunk)`. Returns the payload CRC.
fn payload(
    state: &State,
    extent: (usize, usize),
    scratch: &mut [u8],
    mut visit: impl FnMut(usize, &[u8]) -> Result<()>,
) -> Result<u32> {
    if scratch.len() < VALUE_RECORD {
        return Err(Error::BufferTooSmall);
    }
    let mut crc = CRC_START;
    let mut filled = 0;
    let mut offset = 0;
    let records = (0..extent.0)
        .map(|j| (true, j))
        .chain((0..extent.1).map(|j| (false, j)));
    for (key, j) in records {
        let size = if key { KEY_RECORD } else { VALUE_RECORD };
        if filled + size > scratch.len() {
            crc = crc_step(crc, &scratch[..filled]);
            visit(offset, &scratch[..filled])?;
            offset += filled;
            filled = 0;
        }
        let b = &mut scratch[filled..filled + size];
        if key {
            write_key(b, &state.config.keys[CATALOG_KEYS + j]);
        } else {
            write_value(b, &state.config.values[CATALOG_VALUES + j]);
        }
        filled += size;
    }
    if filled > 0 {
        crc = crc_step(crc, &scratch[..filled]);
        visit(offset, &scratch[..filled])?;
    }
    Ok(!crc)
}
fn header(extent: (usize, usize), crc: u32) -> [u8; HEADER] {
    let mut h = [0; HEADER];
    h[..8].copy_from_slice(b"DOORREG1");
    put32(&mut h, 8, VERSION);
    put32(&mut h, 12, extent.0 as u32);
    put32(&mut h, 16, extent.1 as u32);
    put32(&mut h, 20, crc);
    h
}
/// The header the state's hive would have: equal headers mean equal hives
/// (extent and CRC), so nothing needs writing.
pub(crate) fn identity(state: &State, scratch: &mut [u8]) -> Result<[u8; HEADER]> {
    let extent = extent(state);
    let crc = payload(state, extent, scratch, |_, _| Ok(()))?;
    Ok(header(extent, crc))
}
/// Encode through `write(file offset, bytes)`: the payload in chunks, then
/// the header. Returns the header written.
pub(crate) fn encode(
    state: &State,
    scratch: &mut [u8],
    mut write: impl FnMut(usize, &[u8]) -> Result<()>,
) -> Result<[u8; HEADER]> {
    let extent = extent(state);
    let crc = payload(state, extent, scratch, |offset, chunk| {
        write(HEADER + offset, chunk)
    })?;
    let header = header(extent, crc);
    write(0, &header)?;
    Ok(header)
}
/// Validate a header against the hive file's length; the extent it states.
pub(crate) fn header_extent(header: &[u8], length: usize) -> Result<(usize, usize)> {
    if header.len() < HEADER
        || &header[..8] != b"DOORREG1"
        || get32(header, 8) != VERSION
        || !zero(&header[24..HEADER])
    {
        return Err(Error::Corrupt);
    }
    let extent = (get32(header, 12) as usize, get32(header, 16) as usize);
    if extent.0 > KEYS - CATALOG_KEYS
        || extent.1 > VALUES - CATALOG_VALUES
        || bytes(extent) > length
    {
        return Err(Error::Corrupt);
    }
    Ok(extent)
}
/// Record-aligned payload chunks (payload offset, length) of at most
/// `capacity` bytes for reading `extent`.
pub(crate) fn chunks(
    extent: (usize, usize),
    capacity: usize,
) -> impl Iterator<Item = (usize, usize)> {
    let keys = extent.0;
    let total = keys + extent.1;
    let mut index = 0;
    let mut offset = 0;
    core::iter::from_fn(move || {
        let start = offset;
        while index < total {
            let size = if index < keys {
                KEY_RECORD
            } else {
                VALUE_RECORD
            };
            if offset - start + size > capacity {
                break;
            }
            offset += size;
            index += 1;
        }
        (offset > start).then_some((start, offset - start))
    })
}
/// Decode one record-aligned chunk at payload `offset` into the overflow
/// slots; returns the running CRC (start with `CRC_START`).
pub(crate) fn decode_chunk(
    state: &mut State,
    extent: (usize, usize),
    offset: usize,
    chunk: &[u8],
    crc: u32,
) -> Result<u32> {
    let keys_end = extent.0 * KEY_RECORD;
    let mut at = 0;
    while at < chunk.len() {
        let p = offset + at;
        if p < keys_end {
            let b = chunk.get(at..at + KEY_RECORD).ok_or(Error::Corrupt)?;
            state.config.keys[CATALOG_KEYS + p / KEY_RECORD] = read_key(b)?;
            at += KEY_RECORD;
        } else {
            let j = (p - keys_end) / VALUE_RECORD;
            let b = chunk.get(at..at + VALUE_RECORD).ok_or(Error::Corrupt)?;
            if j >= extent.1 {
                return Err(Error::Corrupt);
            }
            state.config.values[CATALOG_VALUES + j] = read_value(b)?;
            at += VALUE_RECORD;
        }
    }
    Ok(crc_step(crc, chunk))
}
/// The payload CRC read matches the header's.
pub(crate) fn crc_matches(header: &[u8], crc: u32) -> bool {
    get32(header, 20) == !crc
}
