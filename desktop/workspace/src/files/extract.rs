//! "Extract All": a bounded background job (like "Compress to ZIP file")
//! that reads a ZIP, TAR, TAR.GZ or GZ archive whole (up to
//! `MAX_ARCHIVE`), creates a new folder named after it next to it, and
//! writes every safe entry into it a few storage calls per tick. Paths that
//! climb out of the folder, links and devices are never written; encrypted
//! entries need Preview, which asks for the password.
use super::*;
use door_service_client::{File, WRITE};
use door_workspace::archive_codec::{Archive, Fault};

/// Largest archive read for extraction, the most one entry may expand to
/// (well below the 256 MiB native heap) and the most a whole archive may
/// write (decompression bombs stop there).
const MAX_ARCHIVE: u64 = 64 * 1024 * 1024;
const MAX_ENTRY: usize = 64 * 1024 * 1024;
const MAX_TOTAL: u64 = 1024 * 1024 * 1024;
const CALLS_PER_TICK: usize = 64;

enum Step {
    /// Reading the archive (bytes so far).
    Read,
    /// Writing entry `next` (an open output with its data and position).
    Write,
}
pub(super) struct ExtractJob {
    pub(super) name: String,
    pub(super) folder: String,
    source: String,
    file: Option<File>,
    size: u64,
    bytes: Vec<u8>,
    step: Step,
    archive: Option<Archive>,
    next: usize,
    out: Option<(File, Vec<u8>, usize)>,
    made: Vec<String>,
    pub(super) written: usize,
    /// Bytes written so far (bounded by `MAX_TOTAL`).
    total: u64,
    /// Entries left out because a file of that name was already written.
    duplicates: usize,
    /// Percentage last shown in the status bar.
    shown: u64,
}
impl ExtractJob {
    pub(super) fn percent(&self) -> u64 {
        match (&self.step, &self.archive) {
            (Step::Read, _) => (self.bytes.len() as u64 * 30 / self.size.max(1)).min(30),
            (Step::Write, Some(a)) => 30 + (self.next as u64 * 69 / a.items.len().max(1) as u64),
            _ => 0,
        }
    }
}
fn join(parent: &str, name: &str) -> String {
    if parent == "/" {
        format!("/{name}")
    } else {
        format!("{parent}/{name}")
    }
}
/// Archive kinds "Extract All" handles.
pub(super) fn extractable(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    [".zip", ".7z", ".rar", ".tar", ".tgz", ".tar.gz", ".gz"]
        .iter()
        .any(|ext| lower.ends_with(ext))
}
impl Files {
    pub(super) fn begin_extract(&mut self, storage: &mut Storage, source: &str) {
        if self.extract.is_some() {
            self.message = "Another archive is being extracted. Wait for it to finish.".into();
            return;
        }
        let Some((parent, file)) = source.rsplit_once('/') else {
            return;
        };
        let parent = if parent.is_empty() { "/" } else { parent };
        let lower = file.to_ascii_lowercase();
        let stem = [".tar.gz", ".tgz", ".zip", ".7z", ".rar", ".tar", ".gz"]
            .iter()
            .find_map(|ext| {
                lower
                    .ends_with(ext)
                    .then(|| &file[..file.len() - ext.len()])
            })
            .filter(|s| !s.is_empty())
            .unwrap_or(file);
        let info = match storage.metadata(source) {
            Ok(info) if info.bytes <= MAX_ARCHIVE => info,
            Ok(_) => {
                self.message = format!(
                    "Archives larger than {} MB can't be extracted yet.",
                    MAX_ARCHIVE >> 20
                );
                return;
            }
            Err(error) => {
                self.message = format!("Cannot extract {file}: {}", Self::location_error(&error));
                return;
            }
        };
        let taken: Vec<String> = match Self::read_entries(storage, parent) {
            Ok(entries) => entries.iter().map(Self::name).collect(),
            Err(error) => {
                self.message = format!("Cannot extract {file}: {}", Self::location_error(&error));
                return;
            }
        };
        let name = super::archive::unique_name(stem, "", &taken);
        let file_handle = match storage.open(source, Disposition::OpenExisting, READ, SHARE_ALL) {
            Ok(f) => f,
            Err(error) => {
                self.message = format!("Cannot extract {file}: {}", Self::location_error(&error));
                return;
            }
        };
        self.message = format!("Extracting {file}...");
        door_rt::diagnostic(format_args!(
            "DOOR_P15 FILES EXTRACT BEGIN name={name} bytes={}\n",
            info.bytes
        ));
        self.extract = Some(ExtractJob {
            folder: join(parent, &name),
            name,
            source: String::from(source),
            file: Some(file_handle),
            size: info.bytes,
            bytes: Vec::with_capacity(info.bytes as usize),
            step: Step::Read,
            archive: None,
            next: 0,
            out: None,
            made: Vec::new(),
            written: 0,
            total: 0,
            duplicates: 0,
            shown: u64::MAX,
        });
    }
    /// Esc while extracting: stop after the current entry (what was written
    /// stays, as Windows leaves it).
    pub(super) fn cancel_extract(&mut self, storage: &mut Storage) -> bool {
        let Some(mut job) = self.extract.take() else {
            return false;
        };
        if let Some(file) = job.file.take() {
            let _ = storage.close(file);
        }
        if let Some((out, ..)) = job.out.take() {
            let _ = storage.close(out);
        }
        let _ = storage.flush();
        door_rt::diagnostic(format_args!(
            "DOOR_P15 FILES EXTRACT CANCELLED name={} entries={}\n",
            job.name, job.written
        ));
        self.message = format!(
            "Extraction cancelled. {} items were extracted to {}.",
            job.written, job.name
        );
        true
    }
    /// Advances the extraction; returns whether anything visible changed.
    pub(super) fn extract_step(&mut self, storage: &mut Storage) -> bool {
        let Some(mut job) = self.extract.take() else {
            return false;
        };
        match Self::extract_calls(&mut job, storage) {
            Ok(false) => {
                let percent = job.percent();
                let moved = percent != job.shown;
                if moved {
                    job.shown = percent;
                    self.message = format!("Extracting to {}... {percent}%", job.name);
                }
                self.extract = Some(job);
                moved
            }
            Ok(true) => {
                let skipped = job.archive.as_ref().map_or(0, |a| a.skipped) + job.duplicates;
                door_rt::diagnostic(format_args!(
                    "DOOR_P15 FILES EXTRACT DONE name={} entries={} skipped={skipped}\n",
                    job.name, job.written
                ));
                let parent = job
                    .folder
                    .rsplit_once('/')
                    .map_or("/", |(p, _)| if p.is_empty() { "/" } else { p });
                if self.cwd == parent && !self.overview && self.refresh(storage).is_ok() {
                    let found = self.entries.iter().position(|e| Self::name(e) == job.name);
                    if let Some(i) = found {
                        self.selected = Some(i);
                        self.marked.clear();
                    }
                }
                self.message = if skipped > 0 {
                    format!(
                        "Extracted to {} ({skipped} unsafe or unsupported items left out)",
                        job.name
                    )
                } else {
                    format!("Extracted to {}", job.name)
                };
                true
            }
            Err(reason) => {
                if let Some(file) = job.file.take() {
                    let _ = storage.close(file);
                }
                if let Some((out, ..)) = job.out.take() {
                    let _ = storage.close(out);
                }
                door_rt::diagnostic(format_args!(
                    "DOOR_P15 FILES EXTRACT FAILED name={} reason={reason}\n",
                    job.name
                ));
                self.message = format!(
                    "Cannot extract {}: {reason}",
                    job.source.rsplit('/').next().unwrap_or("")
                );
                true
            }
        }
    }
    fn extract_calls(job: &mut ExtractJob, storage: &mut Storage) -> Result<bool, String> {
        let fail = |e: Error| String::from(Self::location_error(&e));
        let mut calls = 0;
        while calls < CALLS_PER_TICK {
            match job.step {
                Step::Read => {
                    let file = job.file.as_ref().ok_or("the archive closed")?;
                    let mut chunk = [0u8; door_service_protocol::CHUNK_BYTES];
                    let n = storage
                        .read(file, job.bytes.len() as u64, &mut chunk)
                        .map_err(fail)?;
                    calls += 1;
                    job.bytes.extend_from_slice(&chunk[..n]);
                    if n > 0 && (job.bytes.len() as u64) < job.size {
                        continue;
                    }
                    let _ = storage.close(job.file.take().unwrap());
                    let bytes = core::mem::take(&mut job.bytes);
                    let archive =
                        Archive::open(&job.source, bytes, MAX_ENTRY).map_err(String::from)?;
                    if archive.items.iter().any(|i| i.encrypted) {
                        return Err("it is password protected. Open it in Preview to extract it with its password.".into());
                    }
                    storage.mkdir(&job.folder).map_err(fail)?;
                    calls += 1;
                    job.made.push(job.folder.clone());
                    job.archive = Some(archive);
                    job.step = Step::Write;
                }
                Step::Write => {
                    let archive = job.archive.as_ref().unwrap();
                    if let Some((out, data, at)) = job.out.as_mut() {
                        if *at < data.len() {
                            let n = (data.len() - *at).min(door_service_protocol::CHUNK_BYTES);
                            let w = storage
                                .write(out, *at as u64, &data[*at..*at + n])
                                .map_err(fail)?;
                            if w == 0 {
                                return Err("the disk refused the data".into());
                            }
                            *at += w;
                            calls += 1;
                            continue;
                        }
                        let (out, ..) = job.out.take().unwrap();
                        storage.close(out).map_err(fail)?;
                        job.written += 1;
                        job.next += 1;
                        continue;
                    }
                    let Some(item) = archive.items.get(job.next) else {
                        storage.flush().map_err(fail)?;
                        return Ok(true);
                    };
                    // Parent folders first, each once.
                    let mut path = job.folder.clone();
                    let parts: Vec<&str> = item.path.split('/').collect();
                    let dirs = if item.folder {
                        parts.len()
                    } else {
                        parts.len() - 1
                    };
                    for part in &parts[..dirs] {
                        path = join(&path, part);
                        if !job.made.contains(&path) {
                            match storage.mkdir(&path) {
                                Ok(()) | Err(Error::Service(Code::Exists)) => (),
                                Err(e) => return Err(fail(e)),
                            }
                            calls += 1;
                            job.made.push(path.clone());
                        }
                    }
                    if item.folder {
                        job.next += 1;
                        continue;
                    }
                    let data = archive.read(job.next, None, MAX_ENTRY).map_err(|f| {
                        String::from(match f {
                            Fault::Password => "an entry is password protected",
                            Fault::Damaged => "the archive is damaged",
                            Fault::Method => "an entry uses a compression method Doors can't read",
                            Fault::TooLarge => "an entry is too large",
                        })
                    })?;
                    job.total += data.len() as u64;
                    if job.total > MAX_TOTAL {
                        return Err(format!("it expands to more than {} GB", MAX_TOTAL >> 30));
                    }
                    let target = join(&path, parts[parts.len() - 1]);
                    let out = match storage.open(&target, Disposition::CreateNew, READ | WRITE, 0) {
                        Ok(out) => out,
                        // A repeated (or case-colliding) name: the first wins.
                        Err(Error::Service(Code::Exists)) => {
                            job.duplicates += 1;
                            job.next += 1;
                            calls += 1;
                            continue;
                        }
                        Err(e) => return Err(fail(e)),
                    };
                    calls += 1;
                    job.out = Some((out, data, 0));
                }
            }
        }
        Ok(false)
    }
}
