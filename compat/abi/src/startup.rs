//! Window/startup fields of `RTL_USER_PROCESS_PARAMETERS` and the derived
//! `STARTUPINFOW` view returned by `GetStartupInfoW`. Pinned SDK offsets only;
//! values that depend on the Windows reference launch are listed in
//! `GUI_FLAGS`/`GUI_SHOW` and `CONSOLE_FLAGS` and confirmed by P13-C.
use crate::{Error, PARAMETERS_ADDRESS, STATE_BYTES};

pub const STARTUP_INFO_BYTES: usize = 104;
/// Prefix of RTL_USER_PROCESS_PARAMETERS holding every field `startup_info` reads.
pub const PARAMETER_FIELDS_BYTES: usize = 0xf0;
const PARAMETERS: usize = 12288;
/// Free storage in the parameters page after the environment block.
const DESKTOP_STORAGE: usize = 2560;
pub const DESKTOP: &str = "WinSta0\\Default";
pub const STARTF_USESHOWWINDOW: u32 = 1;
pub const STARTF_USESTDHANDLES: u32 = 0x100;
pub const SW_SHOWNORMAL: u32 = 1;
/// Desktop (shell) launch of a GUI image: ShellExecute passes nShow.
pub const GUI_FLAGS: u32 = STARTF_USESHOWWINDOW;
pub const GUI_SHOW: u32 = SW_SHOWNORMAL;
/// Console launch by the Door shell: no explicit window or handle flags.
pub const CONSOLE_FLAGS: u32 = 0;

fn put(bytes: &mut [u8], at: usize, value: u64) {
    bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
}
fn get(bytes: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(bytes[at..at + 8].try_into().unwrap())
}
fn get32(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap())
}

/// Fill WindowFlags, ShowWindowFlags, WindowTitle (the image path, as for a
/// launch without an explicit title) and DesktopInfo. GUI processes get no
/// console handles; console processes keep NULL standard-handle fields here
/// because their handles come from `GetStdHandle`'s console adapter.
pub fn prepare(output: &mut [u8], gui: bool) -> Result<(), Error> {
    if output.len() != STATE_BYTES {
        return Err(Error::Size);
    }
    let parameters = &mut output[PARAMETERS..];
    let (flags, show) = if gui {
        (GUI_FLAGS, GUI_SHOW)
    } else {
        (CONSOLE_FLAGS, 0)
    };
    parameters[0xa4..0xa8].copy_from_slice(&flags.to_le_bytes());
    parameters[0xa8..0xac].copy_from_slice(&show.to_le_bytes());
    // WindowTitle repeats the ImagePathName descriptor (length, maximum, buffer).
    let image = get(parameters, 0x60);
    let image_buffer = get(parameters, 0x68);
    put(parameters, 0xb0, image);
    put(parameters, 0xb8, image_buffer);
    let units = DESKTOP.len();
    parameters[0xc0..0xc2].copy_from_slice(&((units * 2) as u16).to_le_bytes());
    parameters[0xc2..0xc4].copy_from_slice(&(((units + 1) * 2) as u16).to_le_bytes());
    put(
        parameters,
        0xc8,
        PARAMETERS_ADDRESS + DESKTOP_STORAGE as u64,
    );
    for (index, unit) in DESKTOP
        .encode_utf16()
        .chain(core::iter::once(0))
        .enumerate()
    {
        let at = DESKTOP_STORAGE + index * 2;
        parameters[at..at + 2].copy_from_slice(&unit.to_le_bytes());
    }
    Ok(())
}

/// `GetStartupInfoW` view of a captured parameters page (at least its first
/// `PARAMETER_FIELDS_BYTES`, where every referenced field lies). Without
/// STARTF_USESTDHANDLES (or hotkey/shell data) Windows reports
/// INVALID_HANDLE_VALUE for the three standard handles.
pub fn startup_info(parameters: &[u8]) -> Result<[u8; STARTUP_INFO_BYTES], Error> {
    if parameters.len() < PARAMETER_FIELDS_BYTES {
        return Err(Error::Size);
    }
    let mut info = [0u8; STARTUP_INFO_BYTES];
    info[..4].copy_from_slice(&(STARTUP_INFO_BYTES as u32).to_le_bytes());
    put(&mut info, 8, get(parameters, 0xd8)); // ShellInfo.Buffer
    put(&mut info, 16, get(parameters, 0xc8)); // DesktopInfo.Buffer
    put(&mut info, 24, get(parameters, 0xb8)); // WindowTitle.Buffer
    info[32..60].copy_from_slice(&parameters[0x88..0xa4]);
    let flags = get32(parameters, 0xa4);
    info[60..64].copy_from_slice(&flags.to_le_bytes());
    info[64..66].copy_from_slice(&(get32(parameters, 0xa8) as u16).to_le_bytes());
    info[66..68].copy_from_slice(&parameters[0xe0..0xe2]); // RuntimeData.Length
    put(&mut info, 72, get(parameters, 0xe8)); // RuntimeData.Buffer
    let handles = flags & (STARTF_USESTDHANDLES | 0x200 | 0x400) != 0;
    for (index, source) in [0x20, 0x28, 0x30].into_iter().enumerate() {
        let value = if handles {
            get(parameters, source)
        } else {
            u64::MAX
        };
        put(&mut info, 80 + index * 8, value);
    }
    Ok(info)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn gui_launch_reports_show_window_desktop_and_title_without_console_handles() {
        let mut state = [0u8; STATE_BYTES];
        crate::initial_state(
            &mut state,
            0x7004_1000,
            0x7000_1000,
            7,
            &"C:\\app.exe".encode_utf16().collect::<std::vec::Vec<_>>(),
            &[34, 65, 34],
        )
        .unwrap();
        prepare(&mut state, true).unwrap();
        let info = startup_info(&state[PARAMETERS..]).unwrap();
        assert_eq!(get32(&info, 0), 104);
        assert_eq!(get32(&info, 60), STARTF_USESHOWWINDOW);
        assert_eq!(u16::from_le_bytes([info[64], info[65]]), 1);
        assert_eq!(get(&info, 16), PARAMETERS_ADDRESS + 2560);
        assert_eq!(get(&info, 24), PARAMETERS_ADDRESS + 256);
        for at in [80, 88, 96] {
            assert_eq!(get(&info, at), u64::MAX);
        }
        let desktop: std::vec::Vec<u16> = state[PARAMETERS + 2560..PARAMETERS + 2560 + 30]
            .chunks_exact(2)
            .map(|b| u16::from_le_bytes([b[0], b[1]]))
            .collect();
        assert_eq!(std::string::String::from_utf16(&desktop).unwrap(), DESKTOP);
        prepare(&mut state, false).unwrap();
        let info = startup_info(&state[PARAMETERS..]).unwrap();
        assert_eq!((get32(&info, 60), info[64]), (0, 0));
        // Explicit standard handles are reported only with their flag.
        let mut page = [0u8; 4096];
        page[0xa4..0xa8].copy_from_slice(&STARTF_USESTDHANDLES.to_le_bytes());
        page[0x28..0x30].copy_from_slice(&0x104u64.to_le_bytes());
        assert_eq!(get(&startup_info(&page).unwrap(), 88), 0x104);
        assert_eq!(startup_info(&page[..100]), Err(Error::Size));
    }
    extern crate std;
}
