//! Programs' own icons (`door_workspace::program_icon`): read in the
//! background, one program at a time in bounded ranges, for the items of the
//! open folder, and drawn wherever File Explorer draws a file's icon.
use super::*;
use door_service_client::{File, FileInfo};
use door_workspace::file_metadata::{Miss, Sparse};
use door_workspace::program_icon::{self, ProgramIcon};

/// Programs whose icons stay decoded (up to ~300 KB each).
const CACHED: usize = 32;
/// Reads per tick, and the most a single icon may fetch.
const CALLS: usize = 32;
const MAX_HELD: u64 = 2 * 1024 * 1024;
const MAX_ROUNDS: u32 = 64;

pub(super) struct IconJob {
    file: File,
    info: FileInfo,
    data: Sparse,
    fetch: Option<(u64, u64)>,
    rounds: u32,
}
impl Files {
    /// The program's own icon, once read (`None`: none, or not read yet).
    pub(super) fn program_icon(&self, info: FileInfo) -> Option<&ProgramIcon> {
        self.program_icons
            .iter()
            .find(|(i, _)| *i == info)
            .and_then(|(_, icon)| icon.as_ref())
    }
    /// Draw a program's own icon in `size` logical px; false when it has
    /// none (the caller draws the type icon).
    pub(super) fn draw_program_icon(
        &self,
        v: &mut ui::View<'_>,
        x: i32,
        y: i32,
        size: u32,
        entry: Option<(&str, FileInfo)>,
    ) -> bool {
        let Some((_, info)) = entry.filter(|(name, _)| is_program(name)) else {
            return false;
        };
        let Some(frame) = self
            .program_icon(info)
            .and_then(|icon| icon.best(size * v.scale))
        else {
            return false;
        };
        v.image_fit(
            (x, y, size, size),
            frame.size as usize,
            frame.size as usize,
            &frame.bgra,
        );
        true
    }
    fn remember_icon(&mut self, info: FileInfo, icon: Option<ProgramIcon>) {
        if self.program_icons.len() == CACHED {
            self.program_icons.remove(0);
        }
        self.program_icons.push((info, icon));
    }
    /// Starts or advances reading the next shown program's icon; true when
    /// one arrived (repaint).
    pub(super) fn icon_step(&mut self, storage: &mut Storage) -> bool {
        if self.icon_job.is_none() {
            let next = self
                .shown()
                .take(500)
                .map(|(_, e)| e)
                .filter(|e| !e.info.directory && is_program(&Self::name(e)))
                .find(|e| !self.program_icons.iter().any(|(i, _)| *i == e.info))
                .map(|e| (self.path_for(e), e.info));
            let Some((path, info)) = next else {
                return false;
            };
            let Some(path) = path else {
                self.remember_icon(info, None);
                return false;
            };
            match storage.open(&path, Disposition::OpenExisting, READ, SHARE_ALL) {
                Ok(file) => {
                    self.icon_job = Some(IconJob {
                        file,
                        info,
                        data: Sparse::new(info.bytes),
                        fetch: None,
                        rounds: 0,
                    });
                }
                Err(_) => {
                    self.remember_icon(info, None);
                    return false;
                }
            }
        }
        let job = self.icon_job.as_mut().unwrap();
        let mut done = None;
        for _ in 0..CALLS {
            if job.fetch.is_none() {
                match program_icon::read(&job.data) {
                    Ok(icon) => {
                        done = Some(icon);
                        break;
                    }
                    Err(Miss::Need(at, len)) => {
                        job.rounds += 1;
                        if job.rounds > MAX_ROUNDS || job.data.held() >= MAX_HELD {
                            done = Some(None);
                            break;
                        }
                        // Headers and directories are small and close
                        // together: fetch at least 4 KiB at a time.
                        job.fetch =
                            Some((at, at.saturating_add(len.max(4096)).min(job.info.bytes)));
                    }
                    Err(Miss::Truncated) => {
                        done = Some(None);
                        break;
                    }
                }
            }
            if let Some((at, end)) = job.fetch {
                let mut chunk = [0u8; door_service_protocol::CHUNK_BYTES];
                let want = (end - at).min(chunk.len() as u64) as usize;
                match storage.read(&job.file, at, &mut chunk[..want]) {
                    Ok(n) if n > 0 => {
                        job.data.insert(at, &chunk[..n]);
                        let next = at + n as u64;
                        job.fetch = (next < end).then_some((next, end));
                    }
                    _ => {
                        done = Some(None);
                        break;
                    }
                }
            }
        }
        let Some(icon) = done else {
            return false;
        };
        let job = self.icon_job.take().unwrap();
        let _ = storage.close(job.file);
        let found = icon.is_some();
        self.remember_icon(job.info, icon);
        found
    }
}
