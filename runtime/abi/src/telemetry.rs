//! Sanitized read-only Monitor snapshot. No pointers, handles, paths or commands.
use crate::Status;
/// The kernel's process table (`door_kernel::executive::MAX_PROCESSES`):
/// 32 since 13 November 2026 (13 before).
pub const MAX_PROCESSES: usize = 32;
/// `Process::mode` bit of a 32-bit (WOW64) Windows program (P17); the rest
/// of the word is the launch mode.
pub const MODE_X86: u32 = 1 << 31;
pub const LEGACY_BYTES: usize = 608;
/// 4: thirty-two slots (3 had thirteen, 2 twelve).
pub const VERSION: u32 = 4;
pub const BYTES: usize = 96 + MAX_PROCESSES * 64;
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Process {
    pub pid: u64,
    pub instance: u64,
    pub manifest: u32,
    pub uid: u32,
    pub mode: u32,
    pub state: u32,
    pub threads: u32,
    pub samples: u64,
    pub private_bytes: u64,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Snapshot {
    pub ticks: u64,
    pub cpus: u32,
    pub total_samples: u64,
    pub busy_samples: u64,
    pub managed_pages: u64,
    pub allocated_pages: u64,
    pub reads: u64,
    pub writes: u64,
    pub flushes: u64,
    pub count: usize,
    pub processes: [Process; MAX_PROCESSES],
}
fn get(b: &[u8], p: usize) -> u64 {
    u64::from_le_bytes(b[p..p + 8].try_into().unwrap())
}
fn word(b: &[u8], p: usize) -> u32 {
    u32::from_le_bytes(b[p..p + 4].try_into().unwrap())
}
fn put(b: &mut [u8], p: usize, v: u64) {
    b[p..p + 8].copy_from_slice(&v.to_le_bytes())
}
fn putword(b: &mut [u8], p: usize, v: u32) {
    b[p..p + 4].copy_from_slice(&v.to_le_bytes())
}
impl Snapshot {
    fn valid(&self) -> bool {
        self.count <= MAX_PROCESSES
            && (1..=4).contains(&self.cpus)
            && self.busy_samples <= self.total_samples
            && self.managed_pages != 0
            && self.managed_pages <= u64::MAX / 4096
            && self.allocated_pages <= self.managed_pages
            && self.processes[..self.count]
                .iter()
                .enumerate()
                .all(|(i, p)| {
                    p.pid != 0
                        && p.instance != 0
                        && crate::platform::manifest_policy(u64::from(p.manifest)).is_some()
                        && p.state <= 6
                        && (1..=16).contains(&p.threads)
                        && p.samples <= self.busy_samples
                        && p.private_bytes.is_multiple_of(4096)
                        && !self.processes[..i]
                            .iter()
                            .any(|q| q.pid == p.pid || q.instance == p.instance)
                })
            && self.processes[self.count..]
                .iter()
                .all(|p| *p == Process::default())
    }
    pub fn encode(self) -> Result<[u8; BYTES], Status> {
        if !self.valid() {
            return Err(Status::InvalidArgument);
        }
        let mut b = [0; BYTES];
        putword(&mut b, 0, VERSION);
        putword(&mut b, 4, self.count as u32);
        put(&mut b, 8, self.ticks);
        putword(&mut b, 16, self.cpus);
        for (p, v) in [
            (24, self.total_samples),
            (32, self.busy_samples),
            (40, self.managed_pages),
            (48, self.allocated_pages),
            (56, self.reads),
            (64, self.writes),
            (72, self.flushes),
        ] {
            put(&mut b, p, v);
        }
        for (i, p) in self.processes[..self.count].iter().enumerate() {
            let at = 96 + i * 64;
            put(&mut b, at, p.pid);
            put(&mut b, at + 8, p.instance);
            for (o, v) in [
                (16, p.manifest),
                (20, p.uid),
                (24, p.mode),
                (28, p.state),
                (32, p.threads),
            ] {
                putword(&mut b, at + o, v)
            }
            put(&mut b, at + 40, p.samples);
            put(&mut b, at + 48, p.private_bytes);
        }
        Ok(b)
    }
    pub fn decode(b: &[u8]) -> Result<Self, Status> {
        if !matches!(b.len(), BYTES | LEGACY_BYTES)
            || word(b, 0) != if b.len() == LEGACY_BYTES { 1 } else { VERSION }
            || word(b, 4)
                > if b.len() == LEGACY_BYTES {
                    8
                } else {
                    MAX_PROCESSES as u32
                }
            || b[20..24].iter().chain(&b[80..96]).any(|v| *v != 0)
        {
            return Err(Status::InvalidArgument);
        }
        let mut s = Self {
            ticks: get(b, 8),
            cpus: word(b, 16),
            total_samples: get(b, 24),
            busy_samples: get(b, 32),
            managed_pages: get(b, 40),
            allocated_pages: get(b, 48),
            reads: get(b, 56),
            writes: get(b, 64),
            flushes: get(b, 72),
            count: word(b, 4) as usize,
            ..Self::default()
        };
        for i in 0..(b.len() - 96) / 64 {
            let at = 96 + i * 64;
            if b[at + 36..at + 40]
                .iter()
                .chain(&b[at + 56..at + 64])
                .any(|v| *v != 0)
            {
                return Err(Status::InvalidArgument);
            }
            s.processes[i] = Process {
                pid: get(b, at),
                instance: get(b, at + 8),
                manifest: word(b, at + 16),
                uid: word(b, at + 20),
                mode: word(b, at + 24),
                state: word(b, at + 28),
                threads: word(b, at + 32),
                samples: get(b, at + 40),
                private_bytes: get(b, at + 48),
            };
        }
        if !s.valid() {
            return Err(Status::InvalidArgument);
        }
        Ok(s)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn sample() -> Snapshot {
        let mut s = Snapshot {
            cpus: 1,
            managed_pages: 1000,
            allocated_pages: 400,
            total_samples: 50,
            busy_samples: 20,
            count: 1,
            ..Snapshot::default()
        };
        s.processes[0] = Process {
            pid: 42,
            instance: 8,
            manifest: 14,
            uid: 1001,
            mode: 11,
            state: 2,
            threads: 1,
            samples: 5,
            private_bytes: 4096,
        };
        s
    }
    #[test]
    fn v4_reports_every_slot_and_decodes_legacy_without_invented_processes() {
        let mut s = sample();
        s.count = MAX_PROCESSES;
        for i in 0..MAX_PROCESSES {
            s.processes[i] = Process {
                pid: 42 + i as u64,
                instance: 80 + i as u64,
                ..s.processes[0]
            };
        }
        // The session IME (manifest 16) is a catalog process; 99 is not.
        s.processes[MAX_PROCESSES - 1].manifest = crate::platform::manifest::IME as u32;
        let bytes = s.encode().unwrap();
        assert_eq!(bytes.len(), 96 + 32 * 64);
        let mut unknown = s;
        unknown.processes[MAX_PROCESSES - 1].manifest = 99;
        assert!(unknown.encode().is_err());
        assert_eq!(Snapshot::decode(&bytes), Ok(s));
        let mut legacy = sample().encode().unwrap()[..LEGACY_BYTES].to_vec();
        legacy[..4].copy_from_slice(&1u32.to_le_bytes());
        let decoded = Snapshot::decode(&legacy).unwrap();
        assert_eq!(decoded, sample());
        legacy[4..8].copy_from_slice(&9u32.to_le_bytes());
        assert!(Snapshot::decode(&legacy).is_err());
        let mut forged = bytes;
        forged[96 + 11 * 64 + 56] = 1;
        assert!(Snapshot::decode(&forged).is_err());
        let mut mismatched = bytes;
        mismatched[..4].copy_from_slice(&1u32.to_le_bytes());
        assert!(Snapshot::decode(&mismatched).is_err());
    }
    #[test]
    fn snapshot_rejects_unknown_reserved_tail_and_forged_totals() {
        let s = sample();
        let good = s.encode().unwrap();
        assert_eq!(Snapshot::decode(&good), Ok(s));
        for p in [0, 20, 80, 132, 152, 160] {
            let mut bad = good;
            bad[p] = 255;
            assert!(Snapshot::decode(&bad).is_err(), "byte{p}");
        }
        assert!(Snapshot::decode(&good[..BYTES - 1]).is_err());
        let mut bad = s;
        bad.busy_samples = 51;
        assert!(bad.encode().is_err());
        bad = s;
        bad.count = 2;
        bad.processes[1] = bad.processes[0];
        assert!(bad.encode().is_err());
    }
}
