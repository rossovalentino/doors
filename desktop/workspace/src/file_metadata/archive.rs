//! Archives: ZIP (central directory, ZIP64, Office/EPUB/JAR detection), TAR
//! and GZIP.
use super::Section::{Archive, Description};
use super::*;

/// Most of a ZIP central directory that is read.
const MAX_DIRECTORY: u64 = 512 * 1024;

fn saved(compressed: u64, original: u64) -> Option<String> {
    (original > 0 && compressed <= original)
        .then(|| format!("{}%", (original - compressed) * 100 / original))
}
fn method(m: u16) -> &'static str {
    match m {
        0 => "Stored",
        8 => "Deflate",
        9 => "Deflate64",
        12 => "BZIP2",
        14 => "LZMA",
        93 => "Zstandard",
        95 => "XZ",
        98 => "PPMd",
        99 => "AES encrypted",
        _ => "Other",
    }
}
/// "2024-05-01 10:20" from MS-DOS date and time fields (no time zone).
fn dos_date(date: u16, time: u16) -> Option<String> {
    let (day, month, year) = (date & 31, (date >> 5) & 15, 1980 + (date >> 9));
    let (minute, hour) = ((time >> 5) & 63, time >> 11);
    ((1..=31).contains(&day) && (1..=12).contains(&month) && hour < 24 && minute < 60)
        .then(|| format!("{year:04}-{month:02}-{day:02} {hour:02}:{minute:02}"))
}

pub(super) fn zip(d: &Sparse, r: &mut Report, ext: &str) -> Result<(), Miss> {
    let size = d.size();
    let tail_len = size.min(65_557);
    let tail = d.get(size - tail_len, tail_len)?;
    let Some(eocd) = (0..tail.len().saturating_sub(21)).rev().find(|i| {
        tail[*i..].starts_with(b"PK\x05\x06")
            && i + 22 + usize::from(le16(tail, i + 20).unwrap_or(0)) <= tail.len()
    }) else {
        r.format("ZIP archive");
        return Err(Miss::Truncated);
    };
    let e = &tail[eocd..];
    let mut entries = u64::from(le16(e, 10).unwrap_or(0));
    let mut cd_size = u64::from(le32(e, 12).unwrap_or(0));
    let mut cd_offset = u64::from(le32(e, 16).unwrap_or(0));
    let comment = e
        .get(22..22 + usize::from(le16(e, 20).unwrap_or(0)))
        .map(latin1)
        .unwrap_or_default();
    // ZIP64: the locator sits just before the end record.
    let eocd_at = size - tail_len + eocd as u64;
    if (entries == 0xFFFF || cd_size == 0xFFFF_FFFF || cd_offset == 0xFFFF_FFFF) && eocd_at >= 20 {
        let locator = d.get(eocd_at - 20, 20)?;
        if locator.starts_with(b"PK\x06\x07") {
            let record = d.get(le64(locator, 8).unwrap_or(u64::MAX).min(size), 56)?;
            if record.starts_with(b"PK\x06\x06") {
                entries = le64(record, 32).unwrap_or(0);
                cd_size = le64(record, 40).unwrap_or(0);
                cd_offset = le64(record, 48).unwrap_or(0);
            }
        }
    }
    let read = cd_size
        .min(MAX_DIRECTORY)
        .min(size.saturating_sub(cd_offset));
    let cd = d.get(cd_offset.min(size), read)?;
    let (mut files, mut folders, mut compressed, mut original) = (0u64, 0u64, 0u64, 0u64);
    let mut encrypted = false;
    let mut methods = [0u64; 11];
    let mut newest: Option<(u16, u16)> = None;
    let mut names = (false, false, false, false, false, false, false);
    let mut at = 0usize;
    let mut walked = 0u64;
    while walked < entries.min(200_000) {
        let Some(h) = cd.get(at..at + 46).filter(|h| h.starts_with(b"PK\x01\x02")) else {
            break;
        };
        let flags = le16(h, 8).unwrap_or(0);
        let m = le16(h, 10).unwrap_or(0);
        let (time, date) = (le16(h, 12).unwrap_or(0), le16(h, 14).unwrap_or(0));
        let mut csize = u64::from(le32(h, 20).unwrap_or(0));
        let mut usize_ = u64::from(le32(h, 24).unwrap_or(0));
        let (nlen, xlen, clen) = (
            usize::from(le16(h, 28).unwrap_or(0)),
            usize::from(le16(h, 30).unwrap_or(0)),
            usize::from(le16(h, 32).unwrap_or(0)),
        );
        let name = cd.get(at + 46..at + 46 + nlen).unwrap_or(&[]);
        let extra = cd.get(at + 46 + nlen..at + 46 + nlen + xlen).unwrap_or(&[]);
        // ZIP64 extended information: the sizes that overflowed, in order.
        let mut x = 0;
        while x + 4 <= extra.len() {
            let (id, len) = (
                le16(extra, x).unwrap_or(0),
                usize::from(le16(extra, x + 2).unwrap_or(0)),
            );
            if id == 1 {
                let mut f = x + 4;
                if usize_ == 0xFFFF_FFFF {
                    usize_ = le64(extra, f).unwrap_or(usize_);
                    f += 8;
                }
                if csize == 0xFFFF_FFFF {
                    csize = le64(extra, f).unwrap_or(csize);
                }
            }
            x += 4 + len;
        }
        if name.ends_with(b"/") {
            folders += 1;
        } else {
            files += 1;
            compressed = compressed.saturating_add(csize);
            original = original.saturating_add(usize_);
            let slot = [0u16, 8, 9, 12, 14, 93, 95, 98, 99]
                .iter()
                .position(|v| *v == m)
                .unwrap_or(9);
            methods[slot] += 1;
        }
        encrypted |= flags & 1 != 0 || m == 99;
        if dos_date(date, time).is_some() && newest.is_none_or(|(d0, t0)| (date, time) > (d0, t0)) {
            newest = Some((date, time));
        }
        let starts = |p: &[u8]| name.starts_with(p);
        names.0 |= starts(b"word/");
        names.1 |= starts(b"xl/");
        names.2 |= starts(b"ppt/");
        names.3 |= name == b"META-INF/MANIFEST.MF";
        names.4 |= name == b"AndroidManifest.xml";
        names.5 |= name == b"META-INF/container.xml";
        names.6 |= name == b"content.xml";
        at += 46 + nlen + xlen + clen;
        walked += 1;
    }
    r.format(match names {
        (true, ..) => "Word document (Office Open XML)",
        (_, true, ..) => "Excel workbook (Office Open XML)",
        (_, _, true, ..) => "PowerPoint presentation (Office Open XML)",
        (_, _, _, _, true, ..) => "Android package",
        (_, _, _, true, ..) => "Java archive",
        (.., true, _) => "EPUB book",
        (.., true) => "OpenDocument file",
        _ if ext == "zip" || ext.is_empty() => "ZIP archive",
        _ => "ZIP-based archive",
    });
    let partial = walked < entries;
    let prefix = if partial { "At least " } else { "" };
    r.add(Archive, "Items", grouped(entries));
    r.add(Archive, "Files", format!("{prefix}{}", grouped(files)));
    r.add(Archive, "Folders", format!("{prefix}{}", grouped(folders)));
    r.add(
        Archive,
        "Uncompressed size",
        format!("{prefix}{}", size_text(original)),
    );
    r.add(
        Archive,
        "Compressed size",
        format!("{prefix}{}", size_text(compressed)),
    );
    if !partial && let Some(s) = saved(compressed, original) {
        r.add(Archive, "Space saved", s);
    }
    if let Some((slot, _)) = methods
        .iter()
        .enumerate()
        .filter(|(_, n)| **n > 0)
        .max_by_key(|(_, n)| **n)
    {
        r.add(
            Archive,
            "Compression method",
            method([0u16, 8, 9, 12, 14, 93, 95, 98, 99, 255, 255][slot]),
        );
    }
    r.add(Archive, "Encrypted", if encrypted { "Yes" } else { "No" });
    if let Some((date, time)) = newest {
        r.add(
            Archive,
            "Newest item",
            dos_date(date, time).unwrap_or_default(),
        );
    }
    r.add(Description, "Comments", comment);
    if partial {
        r.note = Some(LIMITED);
    }
    Ok(())
}
fn size_text(n: u64) -> String {
    super::size(n)
}

fn octal(b: &[u8]) -> Option<u64> {
    if b.first().is_some_and(|c| c & 0x80 != 0) {
        // GNU base-256 for large sizes.
        return Some(b[1..].iter().take(8).fold(0, |v, c| v << 8 | u64::from(*c)));
    }
    let text = b
        .iter()
        .skip_while(|c| **c == b' ')
        .take_while(|c| (b'0'..=b'7').contains(c));
    let mut v = 0u64;
    let mut any = false;
    for c in text.take(22) {
        v = v.checked_mul(8)?.checked_add(u64::from(c - b'0'))?;
        any = true;
    }
    any.then_some(v)
}
pub(super) fn tar(d: &Sparse, r: &mut Report) -> Result<(), Miss> {
    r.format("TAR archive");
    let size = d.size();
    let (mut files, mut folders, mut total, mut zero) = (0u64, 0u64, 0u64, 0);
    let mut at = 0u64;
    let mut kind = "";
    let walk = (|| -> Result<(), Miss> {
        for _ in 0..200_000 {
            if at + 512 > size {
                return Ok(());
            }
            let h = d.get(at, 512)?;
            if h.iter().all(|c| *c == 0) {
                zero += 1;
                if zero == 2 {
                    return Ok(());
                }
                at += 512;
                continue;
            }
            zero = 0;
            let stored = octal(&h[148..156]).ok_or(Miss::Truncated)?;
            let sum: u64 = h
                .iter()
                .enumerate()
                .map(|(i, c)| {
                    if (148..156).contains(&i) {
                        32
                    } else {
                        u64::from(*c)
                    }
                })
                .sum();
            if sum != stored {
                return Err(Miss::Truncated);
            }
            if kind.is_empty() {
                kind = match &h[257..265] {
                    b"ustar\x0000" => "POSIX (ustar)",
                    b"ustar  \0" => "GNU tar",
                    _ => "Original (V7)",
                };
            }
            let len = octal(&h[124..136]).unwrap_or(0);
            match h[156] {
                b'5' => folders += 1,
                b'x' | b'g' | b'L' | b'K' => (),
                _ => {
                    files += 1;
                    total = total.saturating_add(len);
                }
            }
            at = at
                .saturating_add(512)
                .saturating_add(len.div_ceil(512) * 512);
        }
        Ok(())
    })();
    let prefix = if walk.is_ok() { "" } else { "At least " };
    r.add(Archive, "Format", kind);
    r.add(Archive, "Files", format!("{prefix}{}", grouped(files)));
    r.add(Archive, "Folders", format!("{prefix}{}", grouped(folders)));
    r.add(
        Archive,
        "Uncompressed size",
        format!("{prefix}{}", size_text(total)),
    );
    walk
}

pub(super) fn gzip(d: &Sparse, r: &mut Report, name: &str) -> Result<(), Miss> {
    let h = d.get(0, 10)?;
    let flags = h[3];
    let lower = name.to_ascii_lowercase();
    let mut inner_tar = lower.ends_with(".tgz") || lower.ends_with(".tar.gz");
    let mut at = 10u64;
    if flags & 4 != 0 {
        at += 2 + u64::from(d.le16(at)?);
    }
    for (flag, label) in [(8u8, "Original name"), (16, "Comments")] {
        if flags & flag != 0 {
            let b = d.clip(at, 1024)?;
            let end = b.iter().position(|c| *c == 0).ok_or(Miss::Truncated)?;
            let text = latin1(&b[..end]);
            if flag == 8 {
                inner_tar |= text.to_ascii_lowercase().ends_with(".tar");
                r.add(Archive, label, text);
            } else {
                r.add(Description, label, text);
            }
            at += end as u64 + 1;
        }
    }
    r.format(if inner_tar {
        "Compressed TAR archive (gzip)"
    } else {
        "GZIP archive"
    });
    if h[2] != 8 {
        r.add(Archive, "Compression method", "Unknown");
    } else {
        r.add(Archive, "Compression method", "Deflate");
    }
    if let Some(when) = plausible_date(i64::from(le32(h, 4).unwrap_or(0))) {
        r.add(Archive, "Modified (in archive)", when);
    }
    r.add(
        Archive,
        "Created on",
        match h[9] {
            0 => "MS-DOS / Windows (FAT)",
            3 => "Unix",
            7 => "Macintosh",
            11 => "Windows (NTFS)",
            _ => "",
        },
    );
    if d.size() >= 18 {
        let original = u64::from(d.le32(d.size() - 4)?);
        r.add(
            Archive,
            "Uncompressed size",
            if d.size() > u64::from(u32::MAX) {
                String::from("More than 4 GB")
            } else {
                size_text(original)
            },
        );
        r.add(Archive, "Compressed size", size_text(d.size()));
        if let Some(s) = saved(d.size(), original) {
            r.add(Archive, "Space saved", s);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::testing::*;
    use super::*;
    fn zip_file(names: &[(&str, u64, u64, u16)], comment: &str) -> Vec<u8> {
        let mut out = Vec::new();
        let mut cd = Vec::new();
        for (name, csize, usize_, flags) in names {
            let offset = out.len() as u64;
            out.extend(b"PK\x03\x04");
            out.extend([0; 26]);
            out.extend(name.as_bytes());
            out.extend(alloc::vec![0; *csize as usize]);
            cd.extend(b"PK\x01\x02");
            cd.extend(le(20, 2));
            cd.extend(le(20, 2));
            cd.extend(le(u64::from(*flags), 2));
            cd.extend(le(8, 2));
            cd.extend(le(0x5280, 2)); // 10:20
            cd.extend(le(0x58A1, 2)); // 2024-05-01
            cd.extend([0; 4]);
            cd.extend(le(*csize, 4));
            cd.extend(le(*usize_, 4));
            cd.extend(le(name.len() as u64, 2));
            cd.extend([0; 8]);
            cd.extend(le(0, 4));
            cd.extend(le(offset, 4));
            cd.extend(name.as_bytes());
        }
        let cd_offset = out.len() as u64;
        out.extend(&cd);
        out.extend(b"PK\x05\x06");
        out.extend([0; 4]);
        out.extend(le(names.len() as u64, 2));
        out.extend(le(names.len() as u64, 2));
        out.extend(le(cd.len() as u64, 4));
        out.extend(le(cd_offset, 4));
        out.extend(le(comment.len() as u64, 2));
        out.extend(comment.as_bytes());
        out
    }
    #[test]
    fn zip_counts_sizes_ratio_and_office_detection() {
        let z = zip_file(
            &[
                ("docs/", 0, 0, 0),
                ("docs/a.txt", 300, 1000, 0),
                ("b.bin", 700, 1000, 1),
            ],
            "Backup set",
        );
        let r = run("backup.zip", &z);
        assert_eq!(r.format, Some("ZIP archive"));
        assert_eq!(
            (r.get("Items"), r.get("Files"), r.get("Folders")),
            (Some("3"), Some("2"), Some("1"))
        );
        assert_eq!(
            (r.get("Uncompressed size"), r.get("Compressed size")),
            (Some("1.9 KB (2,000 bytes)"), Some("1,000 bytes"))
        );
        assert_eq!(
            (
                r.get("Space saved"),
                r.get("Compression method"),
                r.get("Encrypted")
            ),
            (Some("50%"), Some("Deflate"), Some("Yes"))
        );
        assert_eq!(
            (r.get("Comments"), r.get("Newest item")),
            (Some("Backup set"), Some("2024-05-01 10:20"))
        );
        fuzz("backup.zip", &z);
        let docx = zip_file(
            &[
                ("[Content_Types].xml", 10, 20, 0),
                ("word/document.xml", 10, 20, 0),
            ],
            "",
        );
        assert_eq!(
            run("report.docx", &docx).format,
            Some("Word document (Office Open XML)")
        );
    }
    fn tar_header(name: &str, size: u64, kind: u8) -> Vec<u8> {
        let mut h = alloc::vec![0u8; 512];
        h[..name.len()].copy_from_slice(name.as_bytes());
        h[124..135].copy_from_slice(format!("{size:011o}").as_bytes());
        h[156] = kind;
        h[257..265].copy_from_slice(b"ustar\x0000");
        h[148..156].copy_from_slice(b"        ");
        let sum: u64 = h.iter().map(|c| u64::from(*c)).sum();
        h[148..155].copy_from_slice(format!("{sum:06o}\0").as_bytes());
        h
    }
    #[test]
    fn tar_entries_and_gzip_header() {
        let mut t = tar_header("dir/", 0, b'5');
        t.extend(tar_header("dir/a.txt", 700, b'0'));
        t.extend(alloc::vec![b'a'; 1024]);
        t.extend(tar_header("b.txt", 5, b'0'));
        t.extend(alloc::vec![b'b'; 512]);
        t.extend(alloc::vec![0; 1024]);
        let r = run("src.tar", &t);
        assert_eq!(
            (
                r.get("Files"),
                r.get("Folders"),
                r.get("Uncompressed size"),
                r.get("Format")
            ),
            (
                Some("2"),
                Some("1"),
                Some("705 bytes"),
                Some("POSIX (ustar)")
            )
        );
        fuzz("src.tar", &t);
        let mut g = alloc::vec![0x1F, 0x8B, 8, 8 | 16];
        g.extend(le(1_714_570_507, 4));
        g.extend([0, 3]);
        g.extend(b"src.tar\0nightly\0");
        g.extend([0; 40]);
        g.extend(le(0, 4));
        g.extend(le(20_480, 4));
        let r = run("src.tar.gz", &g);
        assert_eq!(r.format, Some("Compressed TAR archive (gzip)"));
        assert_eq!(
            (
                r.get("Original name"),
                r.get("Comments"),
                r.get("Created on")
            ),
            (Some("src.tar"), Some("nightly"), Some("Unix"))
        );
        assert_eq!(
            (r.get("Modified (in archive)"), r.get("Uncompressed size")),
            (
                Some("2024-05-01 13:35:07 UTC"),
                Some("20.0 KB (20,480 bytes)")
            )
        );
        fuzz("src.tar.gz", &g);
    }
}
