//! Run as administrator: the consent the window server builds itself from
//! the program's bytes, as Windows' consent service does. The publisher is
//! the Authenticode signer the server verified (`door_authenticode`); any
//! failure (unsigned, modified, untrusted, expired) gives the unknown
//! publisher prompt, Windows' default. The program name is the image's
//! FileDescription (version resource), else its file name. Approval is bound
//! to the image's SHA-256, which Init checks against the bytes it loads.
extern crate alloc;
use crate::apps::Shell;
use crate::uac::{Consent, Icon, Then};
use alloc::{format, string::String, vec::Vec};
use door_tls::verify::Anchor;
use sha2::Digest;

/// What the server learned about a program it is asked to elevate.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Inspection {
    pub program: String,
    pub publisher: Option<String>,
    /// The `door_authenticode` result as `WinVerifyTrust` reports it (0 is
    /// a trusted signature).
    pub trust: u32,
    pub image: [u8; 32],
    /// The program's own icon: its largest DIB frame up to 128 px (Vista's
    /// PNG frames need a decoder the server does not carry).
    pub icon: Option<door_pe_icon::Image>,
}
fn own_icon(bytes: &[u8]) -> Option<door_pe_icon::Image> {
    let mut entries = door_pe_icon::group(bytes).ok()??;
    entries.retain(|e| e.width == e.height && e.width <= 128);
    entries.sort_by_key(|e| core::cmp::Reverse((e.width, e.bits)));
    entries.iter().find_map(|e| {
        let data = door_pe_icon::frame(bytes, e).ok()??;
        (!door_pe_icon::is_png(data)).then(|| door_pe_icon::decode_dib(data))?
    })
}
/// Verify and describe `bytes`, the image at `path`, at Unix time `now`.
pub fn inspect(path: &str, bytes: &[u8], now: i64, anchors: &[Anchor<'_>]) -> Inspection {
    let (publisher, trust) = match door_authenticode::verify(bytes, now, anchors) {
        Ok(signature) => (Some(signature.publisher), 0),
        Err(error) => (None, error.hresult()),
    };
    let program = file_description(bytes).unwrap_or_else(|| String::from(file_name(path)));
    let mut image = [0; 32];
    image.copy_from_slice(&sha2::Sha256::digest(bytes));
    Inspection {
        program,
        publisher,
        trust,
        image,
        icon: own_icon(bytes),
    }
}
/// The prompt for a program; `directory` is the working-folder handle the
/// server keeps until Init answers.
pub fn program_consent(
    path: &str,
    inspection: &Inspection,
    directory: u64,
    downloaded: bool,
) -> Consent {
    let cwd = parent(path);
    let mut consent = Consent::new(
        &inspection.program,
        inspection.publisher.as_deref(),
        &display_path(path),
        Icon::File(crate::apps::FileIcon::for_name(path, false)),
        Then::Program {
            path: String::from(path),
            cwd: String::from(cwd),
            directory,
            image: inspection.image,
            arguments: String::new(),
        },
    );
    consent.downloaded = downloaded;
    consent.image = inspection.icon.clone();
    consent
}
/// The prompt for a batch file: Doors Command Processor is what runs
/// elevated, so its name and publisher are shown, as Windows shows Windows
/// Command Processor for a .bat.
pub fn script_consent(path: &str, downloaded: bool) -> Consent {
    let mut consent = Consent::new(
        "Doors Command Processor",
        Some(crate::uac::PUBLISHER),
        &display_path(path),
        Icon::Shell(Shell::CommandPrompt),
        Then::Script(String::from(path)),
    );
    consent.downloaded = downloaded;
    consent
}
/// A batch file Run as administrator applies to.
pub fn is_script(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    lower.ends_with(".bat") || lower.ends_with(".cmd")
}
fn file_name(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}
fn parent(path: &str) -> &str {
    match path.rsplit_once('/') {
        Some(("", _)) | None => "/",
        Some((folder, _)) => folder,
    }
}
/// The location as File Explorer shows it (C: for the Windows tree and
/// user folders).
pub fn display_path(path: &str) -> String {
    if let Some(tail) = path.strip_prefix("/compat/") {
        format!("C:\\{}", tail.replace('/', "\\"))
    } else if let Some(tail) = path.strip_prefix("/users/") {
        format!("C:\\Users\\{}", tail.replace('/', "\\"))
    } else {
        String::from(path)
    }
}
fn le16(b: &[u8], at: usize) -> Option<usize> {
    Some(usize::from(u16::from_le_bytes(
        b.get(at..at + 2)?.try_into().ok()?,
    )))
}
fn le32(b: &[u8], at: usize) -> Option<usize> {
    Some(u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?) as usize)
}
/// The version resource's FileDescription (first language).
pub fn file_description(b: &[u8]) -> Option<String> {
    let pe = le32(b, 0x3C)?;
    if b.get(pe..pe + 4)? != b"PE\0\0" {
        return None;
    }
    let sections = le16(b, pe + 6)?.min(96);
    let optional = pe + 24;
    let size = le16(b, pe + 20)?;
    let dirs = optional + if le16(b, optional)? == 0x20B { 112 } else { 96 };
    if le32(b, dirs - 4)? <= 2 {
        return None;
    }
    let rva = le32(b, dirs + 16)?;
    let table = optional + size;
    let to_file = |rva: usize| -> Option<usize> {
        (0..sections).find_map(|i| {
            let s = table + i * 40;
            let va = le32(b, s + 12)?;
            let span = le32(b, s + 8)?.max(le32(b, s + 16)?);
            (va <= rva && rva < va + span).then(|| le32(b, s + 20).map(|raw| raw + (rva - va)))?
        })
    };
    let base = to_file(rva).filter(|_| rva != 0)?;
    // Resource tree: RT_VERSION (16), first name, first language.
    let mut node = 0;
    for level in 0..3 {
        let n = (le16(b, base + node + 12)? + le16(b, base + node + 14)?).min(256);
        let entry = (0..n)
            .map(|i| base + node + 16 + i * 8)
            .find(|e| level > 0 || le32(b, *e) == Some(16))?;
        let target = le32(b, entry + 4)?;
        if level < 2 {
            if target & 0x8000_0000 == 0 {
                return None;
            }
            node = target & 0x7FFF_FFFF;
        } else {
            let data = base + (target & 0x7FFF_FFFF);
            let at = to_file(le32(b, data)?)?;
            let len = le32(b, data + 4)?.min(65_536);
            return description(b.get(at..at + len)?);
        }
    }
    None
}
/// VS_VERSIONINFO > StringFileInfo > (table) > FileDescription.
fn description(b: &[u8]) -> Option<String> {
    // A block: wLength, wValueLength, wType, key (UTF-16, NUL), padding,
    // value, padding, children.
    struct Block {
        end: usize,
        key: String,
        value: usize,
        value_bytes: usize,
        children: usize,
    }
    let block = |at: usize, limit: usize| -> Option<Block> {
        let len = le16(b, at)?;
        let value_len = le16(b, at + 2)?;
        let text = le16(b, at + 4)? == 1;
        if len < 6 {
            return None;
        }
        let end = (at + len).min(limit);
        let mut key_end = at + 6;
        while key_end + 1 < end && le16(b, key_end)? != 0 {
            key_end += 2;
        }
        let units: Vec<u16> = b
            .get(at + 6..key_end)?
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        let value = (key_end + 2 + 3) & !3;
        let value_bytes = if text { value_len * 2 } else { value_len };
        Some(Block {
            end,
            key: String::from_utf16_lossy(&units),
            value,
            value_bytes,
            children: (value + value_bytes + 3) & !3,
        })
    };
    let children = |start: usize, end: usize| -> Vec<Block> {
        let mut out = Vec::new();
        let mut at = start;
        while at + 6 <= end && out.len() < 64 {
            let Some(c) = block(at, end) else { break };
            if c.end <= at {
                break;
            }
            at = (c.end + 3) & !3;
            out.push(c);
        }
        out
    };
    let root = block(0, b.len())?;
    if root.key != "VS_VERSION_INFO" {
        return None;
    }
    for info in children(root.children, root.end)
        .into_iter()
        .filter(|c| c.key == "StringFileInfo")
    {
        for table in children(info.children, info.end) {
            for s in children(table.children, table.end) {
                if s.key == "FileDescription" {
                    let units: Vec<u16> = b
                        .get(s.value..(s.value + s.value_bytes).min(b.len()))?
                        .chunks_exact(2)
                        .map(|c| u16::from_le_bytes([c[0], c[1]]))
                        .take_while(|u| *u != 0)
                        .collect();
                    let text = String::from_utf16_lossy(&units);
                    let text = text.trim();
                    return (!text.is_empty()).then(|| String::from(text));
                }
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    extern crate std;
    const SIGNED: &[u8] = include_bytes!("../../../crates/authenticode/tests/data/signed.exe");
    const ROOT: &[u8] = include_bytes!("../../../crates/authenticode/tests/data/root.der");
    /// 2026-11-02.
    const NOW: i64 = 1_793_577_600;
    fn root() -> Anchor<'static> {
        let c = door_tls::x509::parse(ROOT).unwrap();
        Anchor {
            subject: c.subject,
            spki: c.spki,
            name_constraints: None,
        }
    }
    #[test]
    fn a_verified_signature_names_the_publisher_and_binds_the_image() {
        let path = "/users/owner/Downloads/setup.exe";
        let seen = inspect(path, SIGNED, NOW, &[root()]);
        assert_eq!(seen.publisher.as_deref(), Some("Doors Test Publisher"));
        assert_eq!(seen.trust, 0);
        assert_eq!(seen.program, "setup.exe");
        let consent = program_consent(path, &seen, 7, true);
        assert_eq!(consent.location, "C:\\Users\\owner\\Downloads\\setup.exe");
        assert!(consent.downloaded);
        assert_eq!(
            consent.then,
            Then::Program {
                path: String::from(path),
                cwd: String::from("/users/owner/Downloads"),
                directory: 7,
                image: seen.image,
                arguments: String::new(),
            }
        );
        assert_eq!(
            consent.question(),
            "Do you want to allow this app to make changes to your device?"
        );
    }
    #[test]
    fn any_signature_failure_is_an_unknown_publisher() {
        let mut tampered = SIGNED.to_vec();
        tampered[0x205] ^= 1;
        for (bytes, anchors) in [
            (&tampered[..], alloc::vec![root()]),
            (SIGNED, alloc::vec![]),
        ] {
            let seen = inspect("/compat/Tools/x.exe", bytes, NOW, &anchors);
            assert_eq!(seen.publisher, None);
            assert_ne!(seen.trust, 0);
            assert!(
                program_consent("/compat/Tools/x.exe", &seen, 1, false)
                    .question()
                    .contains("unknown publisher")
            );
        }
        // The hash is of the bytes shown, so a swapped file cannot match.
        assert_ne!(
            inspect("/a.exe", &tampered, NOW, &[]).image,
            inspect("/a.exe", SIGNED, NOW, &[]).image
        );
    }
    #[test]
    fn batch_files_elevate_the_command_processor() {
        assert!(is_script("/users/owner/Desktop/Setup.CMD") && !is_script("/x/setup.exe"));
        let consent = script_consent("/users/owner/Desktop/fix.bat", false);
        assert_eq!(consent.program, "Doors Command Processor");
        assert_eq!(consent.publisher.as_deref(), Some("Macrohard Doors"));
        assert_eq!(
            consent.then,
            Then::Script(String::from("/users/owner/Desktop/fix.bat"))
        );
    }
    /// A real program's version resource, when DOOR_FILE_DESCRIPTION names
    /// `path=description` (e.g. Git for Windows' git-bash.exe). `--ignored`.
    #[test]
    #[ignore]
    fn live_file_descriptions() {
        let Ok(item) = std::env::var("DOOR_FILE_DESCRIPTION") else {
            return;
        };
        let (path, want) = item.rsplit_once('=').unwrap();
        assert_eq!(
            file_description(&std::fs::read(path).unwrap()).as_deref(),
            Some(want)
        );
    }
}
