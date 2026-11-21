//! Shell links (.lnk, MS-SHLLINK) as CreateShortcuts writes them: the
//! header, LinkInfo with the target's local path, and the description,
//! working directory and arguments strings (Unicode).
use alloc::{string::String, vec::Vec};

const CLSID_SHELL_LINK: [u8; 16] = [
    0x01, 0x14, 0x02, 0x00, 0x00, 0x00, 0x00, 0x00, 0xc0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x46,
];
const HAS_LINK_INFO: u32 = 0x2;
const HAS_NAME: u32 = 0x4;
const HAS_WORKING_DIR: u32 = 0x10;
const HAS_ARGUMENTS: u32 = 0x20;
const IS_UNICODE: u32 = 0x80;
const FILE_ATTRIBUTE_NORMAL: u32 = 0x80;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Link {
    pub target: String,
    pub arguments: String,
    pub description: String,
    pub working_directory: String,
    /// SW_SHOWNORMAL 1, SW_SHOWMAXIMIZED 3, SW_SHOWMINNOACTIVE 7.
    pub show: u32,
}

const HAS_ID_LIST: u32 = 0x1;
/// {20D04FE0-3AEA-1069-A2D8-08002B30309D}, This PC.
const CLSID_MY_COMPUTER: [u8; 16] = [
    0xe0, 0x4f, 0xd0, 0x20, 0xea, 0x3a, 0x69, 0x10, 0xa2, 0xd8, 0x08, 0x00, 0x2b, 0x30, 0x30, 0x9d,
];
/// The target's item ID list, as SHSimpleIDListFromPath builds it: This
/// PC, the drive, then a Unicode file-system item per component (folders
/// 0x35, the file 0x36), ended by a zero size. None for a path that is not
/// `X:\...`.
fn id_list(path: &str) -> Option<Vec<u8>> {
    let bytes = path.as_bytes();
    if bytes.len() < 3 || !bytes[0].is_ascii_alphabetic() || &bytes[1..3] != b":\\" {
        return None;
    }
    let mut list = Vec::new();
    list.extend_from_slice(&0x14u16.to_le_bytes());
    list.extend_from_slice(&[0x1f, 0x50]);
    list.extend_from_slice(&CLSID_MY_COMPUTER);
    let mut drive = [0u8; 25];
    drive[..2].copy_from_slice(&25u16.to_le_bytes());
    drive[2] = 0x2f;
    drive[3] = bytes[0].to_ascii_uppercase();
    drive[4..6].copy_from_slice(b":\\");
    list.extend_from_slice(&drive);
    let parts: Vec<&str> = path[3..].split('\\').filter(|p| !p.is_empty()).collect();
    for (i, part) in parts.iter().enumerate() {
        let file = i + 1 == parts.len();
        let mut item = Vec::new();
        item.push(if file { 0x36 } else { 0x35 });
        item.push(0);
        item.extend_from_slice(&0u32.to_le_bytes()); // size
        item.extend_from_slice(&0u32.to_le_bytes()); // DOS date and time
        let attributes: u16 = if file { 0x20 } else { 0x10 };
        item.extend_from_slice(&attributes.to_le_bytes());
        for u in part.encode_utf16().chain([0]) {
            item.extend_from_slice(&u.to_le_bytes());
        }
        list.extend_from_slice(&((item.len() + 2) as u16).to_le_bytes());
        list.extend_from_slice(&item);
    }
    list.extend_from_slice(&[0, 0]);
    Some(list)
}

fn counted(s: &str) -> Vec<u8> {
    let units: Vec<u16> = s.encode_utf16().collect();
    let mut b = (units.len() as u16).to_le_bytes().to_vec();
    for u in units {
        b.extend_from_slice(&u.to_le_bytes());
    }
    b
}

impl Link {
    pub fn encode(&self) -> Vec<u8> {
        let id_list = id_list(&self.target);
        let mut flags = HAS_LINK_INFO | IS_UNICODE;
        if id_list.is_some() {
            flags |= HAS_ID_LIST;
        }
        if !self.description.is_empty() {
            flags |= HAS_NAME;
        }
        if !self.working_directory.is_empty() {
            flags |= HAS_WORKING_DIR;
        }
        if !self.arguments.is_empty() {
            flags |= HAS_ARGUMENTS;
        }
        let mut b = Vec::new();
        b.extend_from_slice(&0x4cu32.to_le_bytes());
        b.extend_from_slice(&CLSID_SHELL_LINK);
        b.extend_from_slice(&flags.to_le_bytes());
        b.extend_from_slice(&FILE_ATTRIBUTE_NORMAL.to_le_bytes());
        b.extend_from_slice(&[0; 24]); // creation, access, write times
        b.extend_from_slice(&0u32.to_le_bytes()); // file size
        b.extend_from_slice(&0u32.to_le_bytes()); // icon index
        let show = if self.show == 0 { 1 } else { self.show };
        b.extend_from_slice(&show.to_le_bytes());
        b.extend_from_slice(&[0; 2]); // hot key
        b.extend_from_slice(&[0; 10]); // reserved
        if let Some(list) = &id_list {
            b.extend_from_slice(&(list.len() as u16).to_le_bytes());
            b.extend_from_slice(list);
        }
        // LinkInfo: VolumeIDAndLocalBasePath with the ANSI local path and
        // its Unicode form (header size 0x24).
        let path_ansi: Vec<u8> = self
            .target
            .chars()
            .map(|c| if c.is_ascii() { c as u8 } else { b'?' })
            .chain([0])
            .collect();
        let path_wide: Vec<u8> = self
            .target
            .encode_utf16()
            .chain([0])
            .flat_map(u16::to_le_bytes)
            .collect();
        let volume: Vec<u8> = {
            // VolumeID: size, DRIVE_FIXED, serial, label offset, empty label.
            let mut v = Vec::new();
            v.extend_from_slice(&17u32.to_le_bytes());
            v.extend_from_slice(&3u32.to_le_bytes());
            v.extend_from_slice(&0x2026_2630u32.to_le_bytes());
            v.extend_from_slice(&16u32.to_le_bytes());
            v.push(0);
            v
        };
        // An ASCII path takes the short header Windows itself writes; other
        // paths add the Unicode offsets (header 0x24).
        let unicode = !self.target.is_ascii();
        let header = if unicode { 0x24usize } else { 0x1c };
        let volume_at = header;
        let base_at = volume_at + volume.len();
        let suffix_at = base_at + path_ansi.len();
        let wide_base_at = suffix_at + 1;
        let wide_suffix_at = wide_base_at + path_wide.len();
        let total = if unicode {
            wide_suffix_at + 2
        } else {
            suffix_at + 1
        };
        let mut info = Vec::with_capacity(total);
        info.extend_from_slice(&(total as u32).to_le_bytes());
        info.extend_from_slice(&(header as u32).to_le_bytes());
        info.extend_from_slice(&1u32.to_le_bytes()); // VolumeIDAndLocalBasePath
        info.extend_from_slice(&(volume_at as u32).to_le_bytes());
        info.extend_from_slice(&(base_at as u32).to_le_bytes());
        info.extend_from_slice(&0u32.to_le_bytes()); // no network share
        info.extend_from_slice(&(suffix_at as u32).to_le_bytes());
        if unicode {
            info.extend_from_slice(&(wide_base_at as u32).to_le_bytes());
            info.extend_from_slice(&(wide_suffix_at as u32).to_le_bytes());
        }
        info.extend_from_slice(&volume);
        info.extend_from_slice(&path_ansi);
        info.push(0); // empty common path suffix
        if unicode {
            info.extend_from_slice(&path_wide);
            info.extend_from_slice(&[0, 0]);
        }
        b.extend_from_slice(&info);
        if !self.description.is_empty() {
            b.extend_from_slice(&counted(&self.description));
        }
        if !self.working_directory.is_empty() {
            b.extend_from_slice(&counted(&self.working_directory));
        }
        if !self.arguments.is_empty() {
            b.extend_from_slice(&counted(&self.arguments));
        }
        b.extend_from_slice(&0u32.to_le_bytes()); // terminal block
        b
    }
    /// Read a link this module (or Windows) wrote: its local target path,
    /// arguments, description and working directory.
    pub fn decode(b: &[u8]) -> Option<Self> {
        let u32_at = |at: usize| -> Option<u32> {
            Some(u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?))
        };
        if u32_at(0)? != 0x4c || b.get(4..20)? != CLSID_SHELL_LINK {
            return None;
        }
        let flags = u32_at(20)?;
        let show = u32_at(60)?;
        let mut at = 76;
        if flags & 0x1 != 0 {
            // LinkTargetIDList: skipped.
            let size = u16::from_le_bytes(b.get(at..at + 2)?.try_into().ok()?) as usize;
            at += 2 + size;
        }
        let mut target = String::new();
        if flags & HAS_LINK_INFO != 0 {
            let start = at;
            let size = u32_at(start)? as usize;
            let header = u32_at(start + 4)? as usize;
            let info_flags = u32_at(start + 8)?;
            if info_flags & 1 != 0 {
                if header >= 0x24 {
                    let wide = u32_at(start + 28)? as usize;
                    let units: Vec<u16> = b
                        .get(start + wide..start + size)?
                        .chunks_exact(2)
                        .map(|c| u16::from_le_bytes([c[0], c[1]]))
                        .take_while(|u| *u != 0)
                        .collect();
                    target = String::from_utf16(&units).ok()?;
                } else {
                    let base = u32_at(start + 16)? as usize;
                    let bytes = b.get(start + base..start + size)?;
                    let end = bytes.iter().position(|c| *c == 0)?;
                    target = bytes[..end].iter().map(|c| *c as char).collect();
                }
            }
            at = start + size;
        }
        let mut read = |present: bool| -> Option<String> {
            if !present {
                return Some(String::new());
            }
            let n = u16::from_le_bytes(b.get(at..at + 2)?.try_into().ok()?) as usize;
            let units: Vec<u16> = b
                .get(at + 2..at + 2 + 2 * n)?
                .chunks_exact(2)
                .map(|c| u16::from_le_bytes([c[0], c[1]]))
                .collect();
            at += 2 + 2 * n;
            String::from_utf16(&units).ok()
        };
        let description = read(flags & HAS_NAME != 0)?;
        // A relative path string comes between the name and the working
        // directory.
        let _relative = read(flags & 0x8 != 0)?;
        let working_directory = read(flags & HAS_WORKING_DIR != 0)?;
        let arguments = read(flags & HAS_ARGUMENTS != 0)?;
        Some(Self {
            target,
            arguments,
            description,
            working_directory,
            show,
        })
    }
}
