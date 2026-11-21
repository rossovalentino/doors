//! Bounded original redo transactions around a separate standard ext2 partition.
//! This is not JBD/JBD2 or an ext4 on-disk journal.
use super::{
    BLOCK_BYTES, BlockDevice, Error, Geometry, Partition, Result, check_transfer, crc32, put32,
    u32le, u64le,
};

pub const JOURNAL_PAGES: usize = 64;
const SECTORS_PER_PAGE: u64 = 8;
const PAYLOAD_SECTOR: u64 = 16;
const ENTRY_OFFSET: usize = 128;
const CRC_OFFSET: usize = 120;
const COMMITTED: u32 = 1;

/// Initialize directly in static storage in the guest. Never place this value on
/// a kernel stack or copy it into a journal. One exclusive borrow owns all pages.
pub struct WorkingSet {
    pages: [[u8; BLOCK_BYTES]; JOURNAL_PAGES],
    headers: [[u8; BLOCK_BYTES]; 2],
    targets: [u32; JOURNAL_PAGES],
    checksums: [u32; JOURNAL_PAGES],
    count: usize,
}
impl WorkingSet {
    pub const fn new() -> Self {
        Self {
            pages: [[0; BLOCK_BYTES]; JOURNAL_PAGES],
            headers: [[0; BLOCK_BYTES]; 2],
            targets: [0; JOURNAL_PAGES],
            checksums: [0; JOURNAL_PAGES],
            count: 0,
        }
    }
}
impl Default for WorkingSet {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Checkpoint {
    PrepareFlushed,
    CommitFlushed,
    FirstHomeFlushed,
    HomeFlushed,
    CheckpointFlushed,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RecoveryReport {
    pub generation: u64,
    pub pages: usize,
    pub replayed: bool,
    pub repaired_header: bool,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CommitReport {
    pub generation: u64,
    pub pages: usize,
}
#[derive(Clone, Copy)]
struct Header {
    state: u32,
    generation: u64,
    count: usize,
}

/// An exclusively owned device and borrowed staging area. `flush` on this block
/// view is only a staging barrier. Only `commit` acknowledges durable changes.
/// All ext2/cache borrows must end before begin/abort/commit/reopen.
pub struct Journal<'a, D> {
    device: D,
    home: Partition,
    log: Partition,
    disk_id: [u8; 16],
    work: &'a mut WorkingSet,
    generation: u64,
    active: bool,
    poisoned: bool,
}
impl<'a, D: BlockDevice> Journal<'a, D> {
    pub fn open(
        device: D,
        home: Partition,
        log: Partition,
        disk_id: [u8; 16],
        work: &'a mut WorkingSet,
    ) -> Result<(Self, RecoveryReport)> {
        Self::check_layout(device.geometry(), home, log, disk_id)?;
        work.count = 0;
        let mut journal = Self {
            device,
            home,
            log,
            disk_id,
            work,
            generation: 0,
            active: false,
            poisoned: true,
        };
        let report = journal.recover()?;
        journal.poisoned = false;
        Ok((journal, report))
    }

    fn check_layout(g: Geometry, home: Partition, log: Partition, disk: [u8; 16]) -> Result<()> {
        if g.sector_size != 512 || !g.flush_supported {
            return Err(Error::Unsupported);
        }
        if g.read_only || (home.attributes | log.attributes) & (1 << 60) != 0 {
            return Err(Error::ReadOnly);
        }
        if disk == [0; 16] || home.id == [0; 16] || log.id == [0; 16] || home.id == log.id {
            return Err(Error::Corrupt);
        }
        let home_end = home
            .first
            .checked_add(home.sectors)
            .ok_or(Error::Overflow)?;
        let log_end = log.first.checked_add(log.sectors).ok_or(Error::Overflow)?;
        if home.sectors == 0
            || !home.sectors.is_multiple_of(SECTORS_PER_PAGE)
            || home.sectors / SECTORS_PER_PAGE > u64::from(u32::MAX)
            || log.sectors < PAYLOAD_SECTOR + JOURNAL_PAGES as u64 * SECTORS_PER_PAGE
            || home_end > g.sectors
            || log_end > g.sectors
            || home.first < log_end && log.first < home_end
        {
            return Err(Error::Bounds);
        }
        Ok(())
    }
    fn healthy(&self) -> Result<()> {
        if self.poisoned {
            Err(Error::Poisoned)
        } else {
            Ok(())
        }
    }
    pub fn active(&self) -> bool {
        self.active
    }
    pub fn is_poisoned(&self) -> bool {
        self.poisoned
    }
    pub fn staged_pages(&self) -> usize {
        self.work.count
    }
    pub fn generation(&self) -> u64 {
        self.generation
    }
    pub fn begin(&mut self) -> Result<()> {
        self.healthy()?;
        if self.active {
            return Err(Error::Busy);
        }
        if self.generation == u64::MAX {
            return Err(Error::NoSpace);
        }
        self.work.count = 0;
        self.active = true;
        Ok(())
    }
    /// Discard only a healthy, unpublished transaction. Uncertain commits require
    /// dropping the controller and opening it again, never an in-memory abort.
    pub fn abort(&mut self) -> Result<()> {
        self.healthy()?;
        if !self.active {
            return Err(Error::Busy);
        }
        self.work.count = 0;
        self.active = false;
        Ok(())
    }
    pub fn into_inner(self) -> Result<D> {
        self.healthy()?;
        if self.active {
            return Err(Error::Busy);
        }
        Ok(self.device)
    }
    pub fn commit(&mut self) -> Result<CommitReport> {
        self.commit_with(|_| {})
    }
    /// Callback boundaries are for diagnostics that terminate the guest, not for
    /// reentering the journal. Any I/O failure leaves the commit outcome uncertain
    /// and poisons this controller until a fresh open performs recovery.
    pub fn commit_with(&mut self, mut checkpoint: impl FnMut(Checkpoint)) -> Result<CommitReport> {
        self.healthy()?;
        if !self.active {
            return Err(Error::Busy);
        }
        self.active = false;
        if self.work.count == 0 {
            return Ok(CommitReport {
                generation: self.generation,
                pages: 0,
            });
        }
        self.poisoned = true;
        self.compact_changes()?;
        if self.work.count == 0 {
            self.poisoned = false;
            return Ok(CommitReport {
                generation: self.generation,
                pages: 0,
            });
        }
        let generation = self.generation.checked_add(1).ok_or(Error::NoSpace)?;
        let pages = self.work.count;
        for i in 0..pages {
            self.work.checksums[i] = crc32(&self.work.pages[i]);
            self.device
                .write_sectors(self.payload_lba(i), &self.work.pages[i])?;
        }
        self.device.flush()?;
        checkpoint(Checkpoint::PrepareFlushed);
        self.encode(COMMITTED, generation);
        // Flush each mirror separately: a torn second copy retains the first
        // durable commit; a torn first copy retains the previous checkpoint.
        self.publish(0)?;
        self.publish(1)?;
        checkpoint(Checkpoint::CommitFlushed);
        self.apply(&mut checkpoint)?;
        self.encode(0, generation);
        self.publish(0)?;
        self.publish(1)?;
        checkpoint(Checkpoint::CheckpointFlushed);
        self.generation = generation;
        self.work.count = 0;
        self.poisoned = false;
        Ok(CommitReport { generation, pages })
    }
    /// The exclusive controller has performed no home writes since its last
    /// completed checkpoint. Compare final overlay bytes to that durable home
    /// state before publishing any payload. Reuse static header scratch rather
    /// than adding a page to the call stack. Retained targets keep staging order.
    fn compact_changes(&mut self) -> Result<()> {
        let mut retained = 0;
        for index in 0..self.work.count {
            let lba = self.home.first + u64::from(self.work.targets[index]) * SECTORS_PER_PAGE;
            self.device.read_sectors(lba, &mut self.work.headers[0])?;
            if self.work.pages[index] == self.work.headers[0] {
                continue;
            }
            if retained != index {
                let (earlier, current) = self.work.pages.split_at_mut(index);
                earlier[retained].copy_from_slice(&current[0]);
                self.work.targets[retained] = self.work.targets[index];
            }
            retained += 1;
        }
        self.work.count = retained;
        Ok(())
    }
    fn payload_lba(&self, index: usize) -> u64 {
        self.log.first + PAYLOAD_SECTOR + index as u64 * SECTORS_PER_PAGE
    }
    fn apply(&mut self, checkpoint: &mut impl FnMut(Checkpoint)) -> Result<()> {
        for i in 0..self.work.count {
            let lba = self.home.first + u64::from(self.work.targets[i]) * SECTORS_PER_PAGE;
            self.device.write_sectors(lba, &self.work.pages[i])?;
            if i == 0 {
                self.device.flush()?;
                checkpoint(Checkpoint::FirstHomeFlushed);
            }
        }
        self.device.flush()?;
        checkpoint(Checkpoint::HomeFlushed);
        Ok(())
    }
    fn publish(&mut self, slot: usize) -> Result<()> {
        self.device.write_sectors(
            self.log.first + slot as u64 * SECTORS_PER_PAGE,
            &self.work.headers[0],
        )?;
        self.device.flush()
    }
    fn encode(&mut self, state: u32, generation: u64) {
        let b = &mut self.work.headers[0];
        b.fill(0);
        b[..8].copy_from_slice(b"DOORJNL1");
        put32(b, 8, 1);
        put32(b, 12, BLOCK_BYTES as u32);
        put32(b, 16, state);
        put32(
            b,
            20,
            if state == COMMITTED {
                self.work.count as u32
            } else {
                0
            },
        );
        b[24..32].copy_from_slice(&generation.to_le_bytes());
        b[32..48].copy_from_slice(&self.disk_id);
        b[48..64].copy_from_slice(&self.home.id);
        b[64..80].copy_from_slice(&self.log.id);
        for (offset, value) in [
            (80, self.home.first),
            (88, self.home.sectors),
            (96, self.log.first),
            (104, self.log.sectors),
        ] {
            b[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
        }
        put32(b, 112, 512);
        put32(b, 116, BLOCK_BYTES as u32);
        if state == COMMITTED {
            for i in 0..self.work.count {
                put32(b, ENTRY_OFFSET + i * 8, self.work.targets[i]);
                put32(b, ENTRY_OFFSET + i * 8 + 4, self.work.checksums[i]);
            }
        }
        let checksum = crc32(b);
        put32(b, CRC_OFFSET, checksum);
    }
    /// An invalid checksum may be a torn mirror. A checksum-valid but invalid
    /// structure/identity is corruption and must not be hidden by its peer.
    fn decode(&mut self, slot: usize) -> Result<Option<Header>> {
        let b = &mut self.work.headers[slot];
        let expected = u32le(b, CRC_OFFSET);
        put32(b, CRC_OFFSET, 0);
        let checksum = crc32(b);
        put32(b, CRC_OFFSET, expected);
        if checksum != expected {
            return Ok(None);
        }
        let state = u32le(b, 16);
        let count = u32le(b, 20) as usize;
        let generation = u64le(b, 24);
        if &b[..8] != b"DOORJNL1"
            || u32le(b, 8) != 1
            || u32le(b, 12) != BLOCK_BYTES as u32
            || state > COMMITTED
            || count > JOURNAL_PAGES
            || (state == 0 && count != 0)
            || (state == COMMITTED && (count == 0 || generation == 0))
            || b[32..48] != self.disk_id
            || b[48..64] != self.home.id
            || b[64..80] != self.log.id
            || u64le(b, 80) != self.home.first
            || u64le(b, 88) != self.home.sectors
            || u64le(b, 96) != self.log.first
            || u64le(b, 104) != self.log.sectors
            || u32le(b, 112) != 512
            || u32le(b, 116) != BLOCK_BYTES as u32
            || b[124..128].iter().any(|v| *v != 0)
            || b[ENTRY_OFFSET + count * 8..].iter().any(|v| *v != 0)
        {
            return Err(Error::Corrupt);
        }
        for i in 0..count {
            let target = u32le(b, ENTRY_OFFSET + i * 8);
            if u64::from(target) >= self.home.sectors / SECTORS_PER_PAGE
                || (0..i).any(|j| u32le(b, ENTRY_OFFSET + j * 8) == target)
            {
                return Err(Error::Corrupt);
            }
        }
        Ok(Some(Header {
            state,
            generation,
            count,
        }))
    }
    fn recover(&mut self) -> Result<RecoveryReport> {
        for slot in 0..2 {
            self.device.read_sectors(
                self.log.first + slot as u64 * SECTORS_PER_PAGE,
                &mut self.work.headers[slot],
            )?;
        }
        let a = self.decode(0)?;
        let b = self.decode(1)?;
        let (selected, header) = match (a, b) {
            (None, None) => return Err(Error::Corrupt),
            (Some(h), None) => (0, h),
            (None, Some(h)) => (1, h),
            (Some(a), Some(b)) if a.generation == b.generation => {
                if a.state == b.state && self.work.headers[0] != self.work.headers[1] {
                    return Err(Error::Corrupt);
                }
                if a.state <= b.state { (0, a) } else { (1, b) }
            }
            (Some(a), Some(b)) => {
                let (index, high, low) = if a.generation > b.generation {
                    (0, a, b)
                } else {
                    (1, b, a)
                };
                if low.generation.checked_add(1) != Some(high.generation)
                    || low.state != 0
                    || high.state != COMMITTED
                {
                    return Err(Error::Corrupt);
                }
                (index, high)
            }
        };
        let repair = self.work.headers[0] != self.work.headers[1];
        self.generation = header.generation;
        self.work.count = header.count;
        if header.state == COMMITTED {
            for i in 0..header.count {
                self.work.targets[i] = u32le(&self.work.headers[selected], ENTRY_OFFSET + i * 8);
                self.work.checksums[i] =
                    u32le(&self.work.headers[selected], ENTRY_OFFSET + i * 8 + 4);
                self.device
                    .read_sectors(self.payload_lba(i), &mut self.work.pages[i])?;
                if crc32(&self.work.pages[i]) != self.work.checksums[i] {
                    return Err(Error::Corrupt);
                }
            }
            // No home write occurs before every record and payload passes.
            self.apply(&mut |_| {})?;
        }
        if header.state == COMMITTED || repair {
            self.encode(0, header.generation);
            // Preserve the selected valid header until its peer is durable.
            self.publish(1 - selected)?;
            self.publish(selected)?;
        }
        self.work.count = 0;
        Ok(RecoveryReport {
            generation: header.generation,
            pages: header.count,
            replayed: header.state == COMMITTED,
            repaired_header: repair,
        })
    }
    fn io<T>(&mut self, result: Result<T>) -> Result<T> {
        if result.is_err() {
            self.poisoned = true;
        }
        result
    }
}
impl<D: BlockDevice> BlockDevice for Journal<'_, D> {
    fn geometry(&self) -> Geometry {
        Geometry {
            sector_size: 512,
            sectors: self.home.sectors,
            read_only: false,
            flush_supported: true,
        }
    }
    fn read_sectors(&mut self, lba: u64, output: &mut [u8]) -> Result<()> {
        self.healthy()?;
        let sectors = check_transfer(self.geometry(), lba, output.len())?;
        // Staged sectors come from the overlay; each maximal run of unstaged
        // sectors is one home transfer. Per-sector forwarding made every 4 KiB
        // cache miss eight device requests (eight block syscalls in the guest).
        let staged = |work: &WorkingSet, sector: u64| {
            let page = (sector / SECTORS_PER_PAGE) as u32;
            work.targets[..work.count].iter().position(|v| *v == page)
        };
        let mut sector = 0;
        while sector < sectors {
            let at = sector as usize * 512;
            if let Some(index) = staged(&*self.work, lba + sector) {
                let offset = ((lba + sector) % SECTORS_PER_PAGE) as usize * 512;
                output[at..at + 512].copy_from_slice(&self.work.pages[index][offset..offset + 512]);
                sector += 1;
                continue;
            }
            let mut end = sector + 1;
            while end < sectors && staged(&*self.work, lba + end).is_none() {
                end += 1;
            }
            let result = self.device.read_sectors(
                self.home.first + lba + sector,
                &mut output[at..end as usize * 512],
            );
            self.io(result)?;
            sector = end;
        }
        Ok(())
    }
    fn write_sectors(&mut self, lba: u64, input: &[u8]) -> Result<()> {
        self.healthy()?;
        if !self.active {
            return Err(Error::Busy);
        }
        let sectors = check_transfer(self.geometry(), lba, input.len())?;
        let first = lba / SECTORS_PER_PAGE;
        let last = (lba + sectors - 1) / SECTORS_PER_PAGE;
        let missing = (first..=last)
            .filter(|p| !self.work.targets[..self.work.count].contains(&(*p as u32)))
            .count();
        if missing > JOURNAL_PAGES - self.work.count {
            return Err(Error::NoSpace);
        }
        // Fetch every missing page before mutating any previously staged page.
        // A failed read poisons the transaction; abort cannot erase that evidence.
        for page in first..=last {
            if self.work.targets[..self.work.count].contains(&(page as u32)) {
                continue;
            }
            let index = self.work.count;
            let result = self.device.read_sectors(
                self.home.first + page * SECTORS_PER_PAGE,
                &mut self.work.pages[index],
            );
            self.io(result)?;
            self.work.targets[index] = page as u32;
            self.work.count += 1;
        }
        for (i, source) in input.chunks(512).enumerate() {
            let sector = lba + i as u64;
            let page = (sector / SECTORS_PER_PAGE) as u32;
            let index = self.work.targets[..self.work.count]
                .iter()
                .position(|v| *v == page)
                .ok_or(Error::Corrupt)?;
            let offset = (sector % SECTORS_PER_PAGE) as usize * 512;
            self.work.pages[index][offset..offset + 512].copy_from_slice(source);
        }
        Ok(())
    }
    fn flush(&mut self) -> Result<()> {
        self.healthy()?;
        if !self.active {
            return Err(Error::Busy);
        }
        // All preceding exact writes are already visible in this exclusive RAM
        // transaction. The controller's explicit commit supplies persistence.
        Ok(())
    }
}
