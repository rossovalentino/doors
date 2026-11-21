//! "Compress to ZIP file" / "Create archive" and "Convert": bounded background
//! jobs that advance a fixed number of storage calls per event-loop tick (like
//! the folder-size job), so the window stays responsive. Archives are standard
//! ZIP files (CRC-32, sizes patched into each local header after its data,
//! central directory, no ZIP64): entries up to 16 MiB are deflated when that
//! saves space, larger ones are streamed stored; conversion decodes with the
//! preview codecs and writes PNG (stored deflate) or 24-bit BMP. Output always
//! gets a new unique name next to the source; failures remove partial output.
use super::*;
use door_service_client::{File, FileInfo, WRITE};

/// Storage calls a job may spend per tick.
const CALLS_PER_TICK: usize = 64;
pub(super) const MAX_ENTRIES: usize = 4096;
pub(super) const MAX_INPUT: u64 = 256 * 1024 * 1024;
/// Files up to this size are deflated (read whole first); larger ones are
/// streamed stored so a huge file never sits in memory.
const DEFLATE_LIMIT: u64 = 16 * 1024 * 1024;
const MAX_DEPTH: usize = 32;

const fn crc_table() -> [u32; 256] {
    let mut table = [0u32; 256];
    let mut i = 0;
    while i < 256 {
        let mut c = i as u32;
        let mut k = 0;
        while k < 8 {
            c = if c & 1 != 0 {
                0xedb8_8320 ^ (c >> 1)
            } else {
                c >> 1
            };
            k += 1;
        }
        table[i] = c;
        i += 1;
    }
    table
}
static CRC: [u32; 256] = crc_table();
/// Running CRC-32 (IEEE, as ZIP and PNG use it): start at 0, feed chunks.
pub(super) fn crc32(mut crc: u32, bytes: &[u8]) -> u32 {
    crc = !crc;
    for b in bytes {
        crc = CRC[((crc ^ u32::from(*b)) & 0xff) as usize] ^ (crc >> 8);
    }
    !crc
}
/// MS-DOS date of 1980-01-01: the volume records no modification times.
const DOS_DATE: u16 = (1 << 5) | 1;
fn name_flags(name: &str) -> u16 {
    if name.is_ascii() { 0 } else { 1 << 11 }
}
pub(super) fn local_header(name: &str, crc: u32, size: u32) -> Vec<u8> {
    local_header_with(name, 0, crc, size, size)
}
/// A local header for `method` (0 stored, 8 deflate) with the compressed and
/// original sizes.
pub(super) fn local_header_with(
    name: &str,
    method: u16,
    crc: u32,
    packed: u32,
    size: u32,
) -> Vec<u8> {
    let mut b = Vec::with_capacity(30 + name.len());
    b.extend_from_slice(&0x0403_4b50u32.to_le_bytes());
    b.extend_from_slice(&20u16.to_le_bytes());
    b.extend_from_slice(&name_flags(name).to_le_bytes());
    b.extend_from_slice(&method.to_le_bytes());
    b.extend_from_slice(&0u16.to_le_bytes());
    b.extend_from_slice(&DOS_DATE.to_le_bytes());
    b.extend_from_slice(&crc.to_le_bytes());
    b.extend_from_slice(&packed.to_le_bytes());
    b.extend_from_slice(&size.to_le_bytes());
    b.extend_from_slice(&(name.len() as u16).to_le_bytes());
    b.extend_from_slice(&0u16.to_le_bytes());
    b.extend_from_slice(name.as_bytes());
    b
}
pub(super) fn central_header(
    name: &str,
    crc: u32,
    size: u32,
    directory: bool,
    offset: u32,
) -> Vec<u8> {
    central_header_with(name, 0, crc, size, size, directory, offset)
}
pub(super) fn central_header_with(
    name: &str,
    method: u16,
    crc: u32,
    packed: u32,
    size: u32,
    directory: bool,
    offset: u32,
) -> Vec<u8> {
    let mut b = Vec::with_capacity(46 + name.len());
    b.extend_from_slice(&0x0201_4b50u32.to_le_bytes());
    b.extend_from_slice(&20u16.to_le_bytes());
    b.extend_from_slice(&20u16.to_le_bytes());
    b.extend_from_slice(&name_flags(name).to_le_bytes());
    b.extend_from_slice(&method.to_le_bytes());
    b.extend_from_slice(&0u16.to_le_bytes());
    b.extend_from_slice(&DOS_DATE.to_le_bytes());
    b.extend_from_slice(&crc.to_le_bytes());
    b.extend_from_slice(&packed.to_le_bytes());
    b.extend_from_slice(&size.to_le_bytes());
    b.extend_from_slice(&(name.len() as u16).to_le_bytes());
    b.extend_from_slice(&[0; 8]); // extra, comment, disk, internal attributes
    b.extend_from_slice(&(if directory { 0x10u32 } else { 0x20 }).to_le_bytes());
    b.extend_from_slice(&offset.to_le_bytes());
    b.extend_from_slice(name.as_bytes());
    b
}
pub(super) fn end_record(entries: u16, size: u32, offset: u32) -> Vec<u8> {
    let mut b = Vec::with_capacity(22);
    b.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
    b.extend_from_slice(&[0; 4]);
    b.extend_from_slice(&entries.to_le_bytes());
    b.extend_from_slice(&entries.to_le_bytes());
    b.extend_from_slice(&size.to_le_bytes());
    b.extend_from_slice(&offset.to_le_bytes());
    b.extend_from_slice(&0u16.to_le_bytes());
    b
}
/// The archive a job would write, built in memory (host tests).
#[cfg(test)]
pub(super) fn zip_bytes(entries: &[(&str, bool, &[u8])]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut central = Vec::new();
    for (name, directory, data) in entries {
        let offset = out.len() as u32;
        let crc = crc32(0, data);
        out.extend(local_header(name, crc, data.len() as u32));
        out.extend_from_slice(data);
        central.extend(central_header(
            name,
            crc,
            data.len() as u32,
            *directory,
            offset,
        ));
    }
    let offset = out.len() as u32;
    out.extend_from_slice(&central);
    out.extend(end_record(
        entries.len() as u16,
        central.len() as u32,
        offset,
    ));
    out
}

/// PNG with stored (uncompressed) deflate blocks, RGBA 8-bit.
pub(super) fn encode_png(image: &previewing::Image) -> Vec<u8> {
    let (w, h) = (image.width, image.height);
    let mut raw = Vec::with_capacity(h * (w * 4 + 1));
    for row in image.bgra.chunks_exact(w * 4) {
        raw.push(0);
        for p in row.chunks_exact(4) {
            let a = u32::from(p[3]);
            let un = |c: u8| {
                if a == 0 {
                    0
                } else {
                    ((u32::from(c) * 255 + a / 2) / a).min(255) as u8
                }
            };
            raw.extend_from_slice(&[un(p[2]), un(p[1]), un(p[0]), p[3]]);
        }
    }
    let mut z = alloc::vec![0x78, 0x01];
    let blocks = raw.chunks(65_535).count().max(1);
    for (i, block) in raw.chunks(65_535).enumerate() {
        z.push(u8::from(i + 1 == blocks));
        let len = block.len() as u16;
        z.extend_from_slice(&len.to_le_bytes());
        z.extend_from_slice(&(!len).to_le_bytes());
        z.extend_from_slice(block);
    }
    if raw.is_empty() {
        z.extend_from_slice(&[1, 0, 0, 0xff, 0xff]);
    }
    let (mut a, mut b) = (1u32, 0u32);
    for byte in &raw {
        a = (a + u32::from(*byte)) % 65_521;
        b = (b + a) % 65_521;
    }
    z.extend_from_slice(&((b << 16) | a).to_be_bytes());
    let mut png = Vec::from(&b"\x89PNG\r\n\x1a\n"[..]);
    let mut chunk = |kind: &[u8; 4], data: &[u8]| {
        png.extend_from_slice(&(data.len() as u32).to_be_bytes());
        let start = png.len();
        png.extend_from_slice(kind);
        png.extend_from_slice(data);
        let crc = crc32(0, &png[start..]);
        png.extend_from_slice(&crc.to_be_bytes());
    };
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&(w as u32).to_be_bytes());
    ihdr.extend_from_slice(&(h as u32).to_be_bytes());
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]);
    chunk(b"IHDR", &ihdr);
    chunk(b"IDAT", &z);
    chunk(b"IEND", &[]);
    png
}
/// Uncompressed 24-bit BMP; transparency is composited over white.
pub(super) fn encode_bmp(image: &previewing::Image) -> Vec<u8> {
    let (w, h) = (image.width, image.height);
    let row = (w * 3).div_ceil(4) * 4;
    let size = 54 + row * h;
    let mut b = Vec::with_capacity(size);
    b.extend_from_slice(b"BM");
    b.extend_from_slice(&(size as u32).to_le_bytes());
    b.extend_from_slice(&[0; 4]);
    b.extend_from_slice(&54u32.to_le_bytes());
    b.extend_from_slice(&40u32.to_le_bytes());
    b.extend_from_slice(&(w as i32).to_le_bytes());
    b.extend_from_slice(&(h as i32).to_le_bytes());
    b.extend_from_slice(&1u16.to_le_bytes());
    b.extend_from_slice(&24u16.to_le_bytes());
    b.extend_from_slice(&[0; 4]);
    b.extend_from_slice(&((row * h) as u32).to_le_bytes());
    b.extend_from_slice(&[0x13, 0x0b, 0, 0, 0x13, 0x0b, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    for y in (0..h).rev() {
        let start = b.len();
        for p in image.bgra[y * w * 4..(y + 1) * w * 4].chunks_exact(4) {
            let over = |c: u8| (u32::from(c) + 255 - u32::from(p[3])).min(255) as u8;
            b.extend_from_slice(&[over(p[0]), over(p[1]), over(p[2])]);
        }
        b.resize(start + row, 0);
    }
    b
}

/// First free "stem.ext", "stem (2).ext", ... among `names` (case-insensitive).
pub(super) fn unique_name(stem: &str, ext: &str, taken: &[String]) -> String {
    for n in 1..=999 {
        let name = if n == 1 {
            format!("{stem}{ext}")
        } else {
            format!("{stem} ({n}){ext}")
        };
        if !taken.iter().any(|t| t.eq_ignore_ascii_case(&name)) {
            return name;
        }
    }
    format!("{stem} (copy){ext}")
}

struct Item {
    name: String,
    path: String,
    directory: bool,
    info: FileInfo,
}
enum Phase {
    /// Listing folders below the chosen items.
    Scan,
    /// Writing entry `next` (its source open while copying).
    Write,
    Finish,
}
pub(super) struct ArchiveJob {
    pub(super) folder: String,
    pub(super) name: String,
    out_path: String,
    phase: Phase,
    pending: Vec<(String, String, usize)>,
    listing: Option<(String, String, usize, u64)>,
    items: Vec<Item>,
    next: usize,
    out: Option<File>,
    offset: u64,
    source: Option<(File, u64, u32, u64)>,
    /// The current entry read whole for deflate (files up to
    /// `DEFLATE_LIMIT`; larger ones stream stored).
    buffer: Option<Vec<u8>>,
    central: Vec<u8>,
    pub(super) total: u64,
    pub(super) done: u64,
    /// Percentage last shown, so progress repaints only when it moves.
    shown: u64,
}
impl ArchiveJob {
    pub(super) fn percent(&self) -> u64 {
        match self.phase {
            Phase::Scan => 0,
            _ => (self.done * 100)
                .checked_div(self.total.max(1))
                .unwrap_or(0)
                .min(99),
        }
    }
}
pub(super) enum Target {
    Png,
    Bmp,
}
pub(super) struct ConvertJob {
    pub(super) folder: String,
    pub(super) name: String,
    source: String,
    info: FileInfo,
    target: Target,
    file: Option<File>,
    bytes: Vec<u8>,
    out_path: String,
    out: Option<File>,
    data: Vec<u8>,
    written: usize,
}

fn join(parent: &str, name: &str) -> String {
    if parent == "/" {
        format!("/{name}")
    } else {
        format!("{parent}/{name}")
    }
}
fn write_at(storage: &mut Storage, file: &File, offset: u64, mut data: &[u8]) -> Result<(), Error> {
    let mut at = offset;
    while !data.is_empty() {
        let n = data.len().min(door_service_protocol::CHUNK_BYTES);
        let written = storage.write(file, at, &data[..n])?;
        if written == 0 {
            return Err(Error::Service(Code::Io));
        }
        data = &data[written..];
        at += written as u64;
    }
    Ok(())
}

impl Files {
    /// Archive name and destination: the folder that holds the items.
    pub(super) fn begin_archive(&mut self, storage: &mut Storage, sources: Vec<String>) {
        if self.archive.is_some() {
            self.message = "Another archive is being created. Wait for it to finish.".into();
            return;
        }
        let Some(first) = sources.first().filter(|p| p.as_str() != THIS_PC) else {
            return;
        };
        let folder = match first.rsplit_once('/') {
            Some(("", _)) => "/".into(),
            Some((parent, _)) => {
                if first == "/users" {
                    "/compat".into()
                } else {
                    String::from(parent)
                }
            }
            None => return,
        };
        // Named after the clicked item; a file loses its extension, as in Windows.
        let file = storage.metadata(first).is_ok_and(|info| !info.directory);
        let stem = if first == "/users" {
            "Users"
        } else {
            first.rsplit('/').next().unwrap_or("Archive")
        };
        let stem = stem
            .rsplit_once('.')
            .filter(|(s, _)| !s.is_empty() && file)
            .map_or(stem, |(s, _)| s);
        let taken: Vec<String> = match Self::read_entries(storage, &folder) {
            Ok(entries) => entries.iter().map(Self::name).collect(),
            Err(error) => {
                self.message = format!(
                    "Cannot create the archive: {}",
                    Self::location_error(&error)
                );
                return;
            }
        };
        let name = unique_name(stem, ".zip", &taken);
        let mut job = ArchiveJob {
            out_path: join(&folder, &name),
            folder,
            name,
            phase: Phase::Scan,
            pending: Vec::new(),
            listing: None,
            items: Vec::new(),
            next: 0,
            out: None,
            offset: 0,
            source: None,
            buffer: None,
            central: Vec::new(),
            total: 0,
            done: 0,
            shown: u64::MAX,
        };
        for path in sources {
            let info = match storage.metadata(&path) {
                Ok(info) => info,
                Err(error) => {
                    self.message = format!(
                        "Cannot read {}: {}",
                        path.rsplit('/').next().unwrap_or(""),
                        Self::location_error(&error)
                    );
                    return;
                }
            };
            let label = if path == "/users" {
                "Users".into()
            } else {
                String::from(path.rsplit('/').next().unwrap_or("item"))
            };
            if info.directory {
                job.pending.push((format!("{label}/"), path.clone(), 0));
                job.items.push(Item {
                    name: format!("{label}/"),
                    path,
                    directory: true,
                    info,
                });
            } else {
                job.total += info.bytes;
                job.items.push(Item {
                    name: label,
                    path,
                    directory: false,
                    info,
                });
            }
        }
        self.message = format!("Compressing to {}...", job.name);
        door_rt::diagnostic(format_args!(
            "DOOR_P12 FILES ARCHIVE BEGIN name={} items={}\n",
            job.name,
            job.items.len()
        ));
        self.archive = Some(job);
    }
    /// Advances the archive job; returns whether anything visible changed.
    pub(super) fn archive_step(&mut self, storage: &mut Storage) -> bool {
        let Some(mut job) = self.archive.take() else {
            return false;
        };
        match Self::archive_calls(&mut job, storage) {
            Ok(false) => {
                let percent = job.percent();
                let moved = percent != job.shown;
                job.shown = percent;
                self.archive = Some(job);
                moved
            }
            Ok(true) => {
                let (folder, name) = (job.folder.clone(), job.name.clone());
                door_rt::diagnostic(format_args!(
                    "DOOR_P12 FILES ARCHIVE DONE name={name} bytes={}\n",
                    job.offset
                ));
                if self.cwd == folder && !self.overview && self.refresh(storage).is_ok() {
                    let found = self.entries.iter().position(|e| Self::name(e) == name);
                    if let Some(i) = found {
                        self.selected = Some(i);
                        self.marked.clear();
                        let position = self.shown().position(|(j, _)| j == i);
                        if let Some(p) = position {
                            self.reveal_position(p);
                        }
                    }
                }
                self.message = format!("Created {name}");
                true
            }
            Err(error) => {
                if let Some((file, ..)) = job.source.take() {
                    let _ = storage.close(file);
                }
                if let Some(out) = job.out.take() {
                    let _ = storage.close(out);
                    let _ = storage.remove(&job.out_path);
                    let _ = storage.flush();
                }
                door_rt::diagnostic(format_args!(
                    "DOOR_P12 FILES ARCHIVE FAILED error={error:?}\n"
                ));
                self.message = match error {
                    Error::Service(Code::Resources) => format!(
                        "Cannot create {}: more than {} items or {} MB.",
                        job.name,
                        MAX_ENTRIES,
                        MAX_INPUT >> 20
                    ),
                    Error::Service(Code::Changed | Code::StaleInstance) => {
                        format!(
                            "Cannot create {}: an item changed while it was compressed.",
                            job.name
                        )
                    }
                    other => format!(
                        "Cannot create {}: {}",
                        job.name,
                        Self::location_error(&other)
                    ),
                };
                true
            }
        }
    }
    /// Spends up to `CALLS_PER_TICK` storage calls; `Ok(true)` when complete.
    fn archive_calls(job: &mut ArchiveJob, storage: &mut Storage) -> Result<bool, Error> {
        let mut calls = 0;
        while calls < CALLS_PER_TICK {
            match job.phase {
                Phase::Scan => {
                    if job.listing.is_none() {
                        match job.pending.pop() {
                            Some((name, path, depth)) => job.listing = Some((name, path, depth, 0)),
                            None => {
                                job.phase = Phase::Write;
                                let out = storage.open(
                                    &job.out_path,
                                    Disposition::CreateNew,
                                    READ | WRITE,
                                    0,
                                )?;
                                job.out = Some(out);
                                calls += 1;
                                continue;
                            }
                        }
                    }
                    let (name, path, depth, cookie) = job.listing.as_mut().unwrap();
                    calls += 1;
                    match storage.list(path, *cookie)? {
                        Some(entry) => {
                            *cookie = entry.cookie;
                            let child = String::from_utf16(&entry.name[..entry.name_len])
                                .map_err(|_| Error::Service(Code::Unsupported))?;
                            let child_path = if path.as_str() == "/compat" && child == "Users" {
                                "/users".into()
                            } else {
                                join(path, &child)
                            };
                            if job.items.len() >= MAX_ENTRIES || *depth + 1 >= MAX_DEPTH {
                                return Err(Error::Service(Code::Resources));
                            }
                            // Aliases (C:\Users) and hard links are stored once.
                            if job.items.iter().any(|i| i.info.object == entry.info.object) {
                                continue;
                            }
                            let zip_name = format!(
                                "{name}{child}{}",
                                if entry.info.directory { "/" } else { "" }
                            );
                            if entry.info.directory {
                                job.pending.push((
                                    zip_name.clone(),
                                    child_path.clone(),
                                    *depth + 1,
                                ));
                            } else {
                                job.total += entry.info.bytes;
                                if job.total > MAX_INPUT {
                                    return Err(Error::Service(Code::Resources));
                                }
                            }
                            job.items.push(Item {
                                name: zip_name,
                                path: child_path,
                                directory: entry.info.directory,
                                info: entry.info,
                            });
                        }
                        None => job.listing = None,
                    }
                }
                Phase::Write => {
                    let out = job.out.as_ref().ok_or(Error::Malformed)?;
                    if job.source.is_none() {
                        let Some(item) = job.items.get(job.next) else {
                            job.phase = Phase::Finish;
                            continue;
                        };
                        let header = local_header(&item.name, 0, 0);
                        if job.offset + header.len() as u64 + item.info.bytes > u64::from(u32::MAX)
                        {
                            return Err(Error::Service(Code::Resources));
                        }
                        let header_at = job.offset;
                        calls += header.len().div_ceil(door_service_protocol::CHUNK_BYTES);
                        write_at(storage, out, header_at, &header)?;
                        job.offset += header.len() as u64;
                        if item.directory {
                            job.central.extend(central_header(
                                &item.name,
                                0,
                                0,
                                true,
                                header_at as u32,
                            ));
                            job.next += 1;
                            continue;
                        }
                        let file =
                            storage.open(&item.path, Disposition::OpenExisting, READ, SHARE_ALL)?;
                        calls += 2;
                        if storage.info(&file)? != item.info {
                            let _ = storage.close(file);
                            return Err(Error::Service(Code::Changed));
                        }
                        job.buffer = (item.info.bytes <= DEFLATE_LIMIT)
                            .then(|| Vec::with_capacity(item.info.bytes as usize));
                        job.source = Some((file, 0, 0, header_at));
                        continue;
                    }
                    let item = &job.items[job.next];
                    let (file, read, crc, _) = job.source.as_mut().unwrap();
                    if *read < item.info.bytes {
                        let mut chunk = [0u8; door_service_protocol::CHUNK_BYTES];
                        let want = (item.info.bytes - *read).min(chunk.len() as u64) as usize;
                        let n = storage.read(file, *read, &mut chunk[..want])?;
                        if n == 0 {
                            return Err(Error::Service(Code::Changed));
                        }
                        *crc = crc32(*crc, &chunk[..n]);
                        match job.buffer.as_mut() {
                            Some(buffer) => {
                                buffer.extend_from_slice(&chunk[..n]);
                                calls += 1;
                            }
                            None => {
                                write_at(storage, out, job.offset, &chunk[..n])?;
                                job.offset += n as u64;
                                calls += 2;
                            }
                        }
                        *read += n as u64;
                        job.done += n as u64;
                        continue;
                    }
                    let (file, _, crc, header_at) = job.source.take().unwrap();
                    let actual = storage.info(&file);
                    storage.close(file)?;
                    if actual? != item.info {
                        return Err(Error::Service(Code::Changed));
                    }
                    let size = item.info.bytes as u32;
                    // Deflate when it saves space (ZIP's "normal" level); kept
                    // stored otherwise, as Windows does for incompressible data.
                    let (method, packed) = match job.buffer.take() {
                        Some(raw) => {
                            let deflated = door_workspace::archive_codec::deflate(&raw, 6);
                            let (method, data) = if deflated.len() < raw.len() {
                                (door_workspace::archive_codec::DEFLATE, deflated)
                            } else {
                                (door_workspace::archive_codec::STORED, raw)
                            };
                            calls += data.len().div_ceil(door_service_protocol::CHUNK_BYTES);
                            write_at(storage, out, job.offset, &data)?;
                            job.offset += data.len() as u64;
                            (method, data.len() as u32)
                        }
                        None => (door_workspace::archive_codec::STORED, size),
                    };
                    let mut patch = Vec::with_capacity(12);
                    patch.extend_from_slice(&crc.to_le_bytes());
                    patch.extend_from_slice(&packed.to_le_bytes());
                    patch.extend_from_slice(&size.to_le_bytes());
                    write_at(storage, out, header_at + 8, &method.to_le_bytes())?;
                    write_at(storage, out, header_at + 14, &patch)?;
                    job.central.extend(central_header_with(
                        &item.name,
                        method,
                        crc,
                        packed,
                        size,
                        false,
                        header_at as u32,
                    ));
                    job.next += 1;
                    calls += 3;
                }
                Phase::Finish => {
                    let out = job.out.take().ok_or(Error::Malformed)?;
                    let offset = job.offset;
                    let mut tail = core::mem::take(&mut job.central);
                    let size = tail.len() as u32;
                    tail.extend(end_record(job.items.len() as u16, size, offset as u32));
                    let result = write_at(storage, &out, offset, &tail)
                        .and_then(|()| storage.info(&out))
                        .map(|info| info.bytes == offset + tail.len() as u64);
                    job.offset += tail.len() as u64;
                    let closed = storage.close(out);
                    if !result? {
                        return Err(Error::Service(Code::Io));
                    }
                    closed?;
                    storage.flush()?;
                    return Ok(true);
                }
            }
        }
        Ok(false)
    }
    pub(super) fn convert_target(name: &str) -> Option<(Target, &'static str)> {
        let ext = name.rsplit_once('.').map_or("", |(_, e)| e);
        match Kind::of(name, false) {
            Kind::Image if ext.eq_ignore_ascii_case("png") => Some((Target::Bmp, "Convert to BMP")),
            Kind::Image => Some((Target::Png, "Convert to PNG")),
            _ => None,
        }
    }
    pub(super) fn begin_convert(&mut self, storage: &mut Storage) {
        if self.convert.is_some() {
            return;
        }
        let Some(d) = self.detail.as_ref().filter(|d| !d.info.directory) else {
            return;
        };
        let Some((target, _)) = Self::convert_target(&d.name) else {
            return;
        };
        if d.info.bytes > previewing::MAX_VIEW_INPUT as u64 {
            self.message = "Images larger than 6 MB cannot be converted.".into();
            return;
        }
        let folder = match d.path.rsplit_once('/') {
            Some(("", _)) => String::from("/"),
            Some((p, _)) => p.into(),
            None => return,
        };
        let stem = d.name.rsplit_once('.').map_or(d.name.as_str(), |(s, _)| s);
        let ext = match target {
            Target::Png => ".png",
            Target::Bmp => ".bmp",
        };
        let taken: Vec<String> = match Self::read_entries(storage, &folder) {
            Ok(e) => e.iter().map(Self::name).collect(),
            Err(error) => {
                self.message = format!("Cannot convert: {}", Self::location_error(&error));
                return;
            }
        };
        let name = unique_name(stem, ext, &taken);
        let file = match storage.open(&d.path, Disposition::OpenExisting, READ, SHARE_ALL) {
            Ok(f) => f,
            Err(error) => {
                self.message = format!("Cannot convert: {}", Self::location_error(&error));
                return;
            }
        };
        self.message = format!("Converting to {name}...");
        self.convert = Some(ConvertJob {
            out_path: join(&folder, &name),
            folder,
            name,
            source: d.path.clone(),
            info: d.info,
            target,
            file: Some(file),
            bytes: Vec::new(),
            out: None,
            data: Vec::new(),
            written: 0,
        });
    }
    pub(super) fn convert_step(&mut self, storage: &mut Storage) -> bool {
        let Some(mut job) = self.convert.take() else {
            return false;
        };
        match Self::convert_calls(&mut job, storage) {
            Ok(false) => {
                self.convert = Some(job);
                false
            }
            Ok(true) => {
                if self.cwd == job.folder && !self.overview {
                    let _ = self.refresh(storage);
                }
                self.message = format!("Created {}", job.name);
                door_rt::diagnostic(format_args!(
                    "DOOR_P12 FILES CONVERT DONE name={}\n",
                    job.name
                ));
                true
            }
            Err(message) => {
                if let Some(f) = job.file.take() {
                    let _ = storage.close(f);
                }
                if let Some(out) = job.out.take() {
                    let _ = storage.close(out);
                    let _ = storage.remove(&job.out_path);
                    let _ = storage.flush();
                }
                self.message = format!(
                    "Cannot convert {}: {message}",
                    job.source.rsplit('/').next().unwrap_or("")
                );
                true
            }
        }
    }
    fn convert_calls(job: &mut ConvertJob, storage: &mut Storage) -> Result<bool, String> {
        let fail = |e: Error| String::from(Self::location_error(&e));
        for _ in 0..CALLS_PER_TICK {
            if let Some(file) = job.file.as_ref() {
                if (job.bytes.len() as u64) < job.info.bytes {
                    let mut chunk = [0u8; door_service_protocol::CHUNK_BYTES];
                    let n = storage
                        .read(file, job.bytes.len() as u64, &mut chunk)
                        .map_err(fail)?;
                    if n == 0 {
                        return Err("the file changed while it was read.".into());
                    }
                    job.bytes.extend_from_slice(&chunk[..n]);
                    continue;
                }
                let file = job.file.take().unwrap();
                let info = storage.info(&file);
                let _ = storage.close(file);
                if info.map_err(fail)? != job.info {
                    return Err("the file changed while it was read.".into());
                }
                let image = previewing::decode_full(&job.bytes).map_err(String::from)?;
                job.data = match job.target {
                    Target::Png => encode_png(&image),
                    Target::Bmp => encode_bmp(&image),
                };
                job.bytes = Vec::new();
                job.out = Some(
                    storage
                        .open(&job.out_path, Disposition::CreateNew, READ | WRITE, 0)
                        .map_err(fail)?,
                );
                continue;
            }
            let Some(out) = job.out.as_ref() else {
                return Err("the output could not be created.".into());
            };
            if job.written < job.data.len() {
                let end = (job.written + door_service_protocol::CHUNK_BYTES).min(job.data.len());
                let n = storage
                    .write(out, job.written as u64, &job.data[job.written..end])
                    .map_err(fail)?;
                if n == 0 {
                    return Err("the output could not be written.".into());
                }
                job.written += n;
                continue;
            }
            let out = job.out.take().unwrap();
            storage.close(out).map_err(fail)?;
            storage.flush().map_err(fail)?;
            return Ok(true);
        }
        Ok(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn u16at(b: &[u8], at: usize) -> u16 {
        u16::from_le_bytes([b[at], b[at + 1]])
    }
    fn u32at(b: &[u8], at: usize) -> u32 {
        u32::from_le_bytes(b[at..at + 4].try_into().unwrap())
    }
    #[test]
    fn crc_matches_the_standard_check_value() {
        assert_eq!(crc32(0, b"123456789"), 0xcbf4_3926);
        assert_eq!(crc32(crc32(0, b"1234"), b"56789"), 0xcbf4_3926);
    }
    #[test]
    fn stored_zip_has_consistent_local_central_and_end_records() {
        let zip = zip_bytes(&[
            ("Doors OS/", true, b""),
            ("Doors OS/a.txt", false, b"hello\n"),
            ("b\u{e9}.txt", false, b"x"),
        ]);
        let end = zip.len() - 22;
        assert_eq!(u32at(&zip, end), 0x0605_4b50);
        assert_eq!(u16at(&zip, end + 10), 3);
        let (size, offset) = (
            u32at(&zip, end + 12) as usize,
            u32at(&zip, end + 16) as usize,
        );
        assert_eq!(offset + size, end);
        let mut at = offset;
        for (name, data) in [
            ("Doors OS/", &b""[..]),
            ("Doors OS/a.txt", b"hello\n"),
            ("b\u{e9}.txt", b"x"),
        ] {
            assert_eq!(u32at(&zip, at), 0x0201_4b50);
            let crc = u32at(&zip, at + 16);
            assert_eq!(crc, crc32(0, data));
            assert_eq!(u32at(&zip, at + 24) as usize, data.len());
            let n = u16at(&zip, at + 28) as usize;
            assert_eq!(&zip[at + 46..at + 46 + n], name.as_bytes());
            assert_eq!(u16at(&zip, at + 8) & (1 << 11) != 0, !name.is_ascii());
            let local = u32at(&zip, at + 42) as usize;
            assert_eq!(u32at(&zip, local), 0x0403_4b50);
            assert_eq!(u32at(&zip, local + 14), crc);
            assert_eq!(&zip[local + 30 + n..local + 30 + n + data.len()], data);
            at += 46 + n;
        }
        assert_eq!(
            unique_name(
                "Doors OS",
                ".zip",
                &["doors os.zip".into(), "Doors OS (2).zip".into()]
            ),
            "Doors OS (3).zip"
        );
    }
    #[test]
    fn converted_images_decode_back_to_the_same_pixels() {
        let rgba: Vec<u8> = (0..24u8)
            .flat_map(|i| [i * 9, 255 - i * 7, i * 3, 255])
            .collect();
        let source = qoi::encode_to_vec(&rgba, 6, 4).unwrap();
        let image = previewing::decode_full(&source).unwrap();
        let png = previewing::decode_full(&encode_png(&image)).unwrap();
        assert_eq!((png.width, png.height), (6, 4));
        assert_eq!(png.bgra, image.bgra);
        let bmp = previewing::decode_full(&encode_bmp(&image)).unwrap();
        assert_eq!(bmp.bgra, image.bgra);
        assert_eq!(
            previewing::dimensions(&encode_png(&image)),
            previewing::Header::Size(6, 4)
        );
    }
}
