//! Run as administrator's service side (`door_window::elevation` builds the
//! consent): a Doors app hands over a program and its bytes; the server
//! verifies the signature itself, shows User Account Control and, on
//! approval, has Init start exactly those bytes (`uac_answer`).
use super::*;
use door_abi::platform::EXECUTABLE_MAX_BYTES;
use door_abi::{HandleKind, Transfer, rights};
use door_system_services::server;

/// FILETIME (100 ns since 1601) to Unix seconds.
fn unix_time() -> Option<i64> {
    let filetime = door_sys::wall_time().ok()?;
    i64::try_from(filetime / 10_000_000)
        .ok()?
        .checked_sub(11_644_473_600)
}

impl Desktop {
    /// `window::REQUEST_ELEVATION`; a refusal is logged with its reason.
    pub(super) fn elevation_request(
        &mut self,
        owner: u64,
        manifest: u64,
        b: &[u8],
        t: &[Transfer],
    ) -> Result<Response, Code> {
        let result = self.elevation_inner(manifest, b, t);
        // A program's prompt is open: its outcome is the requester's.
        if result.is_ok()
            && self
                .shell
                .uac
                .as_ref()
                .is_some_and(|c| matches!(c.then, door_window::uac::Then::Program { .. }))
        {
            // An earlier outcome this requester never collected goes.
            self.elevation_forget(owner);
            self.shell.elevation_requester = Some(owner);
        }
        if let Err(code) = &result {
            let _ =
                door_sys::write(format!("DOOR_P16 ELEVATION refused code={code:?}\n").as_bytes());
        }
        result
    }
    fn elevation_inner(
        &mut self,
        manifest: u64,
        b: &[u8],
        t: &[Transfer],
    ) -> Result<Response, Code> {
        // File Explorer, or a Windows program's ShellExecuteEx "runas" (any
        // program may ask, as on Windows; only the user's consent on the
        // secure desktop elevates, and only the image verified here).
        use door_abi::platform::manifest::{WINDOWS, WINDOWS_APPLICATION, WINDOWS_GUI, WORKSPACE};
        // File Explorer, and Windows programs (console or GUI) through
        // ShellExecuteEx's runas verb.
        if !matches!(
            manifest,
            WORKSPACE | WINDOWS | WINDOWS_APPLICATION | WINDOWS_GUI
        ) {
            return Err(Code::Denied);
        }
        if b.len() < 10 {
            return Err(Code::InvalidRequest);
        }
        let flags = u32::from_le_bytes(b[..4].try_into().unwrap());
        let image_bytes = u32::from_le_bytes(b[4..8].try_into().unwrap()) as usize;
        let len = usize::from(u16::from_le_bytes([b[8], b[9]]));
        if b.len() != 10 + len
            || len == 0
            || len > w::elevation::MAX_PATH
            || flags
                & !(w::elevation::SCRIPT
                    | w::elevation::DOWNLOADED
                    | w::elevation::PACKAGE
                    | w::elevation::ARGUMENTS)
                != 0
            || flags & w::elevation::SCRIPT != 0 && flags & w::elevation::PACKAGE != 0
            || flags & w::elevation::ARGUMENTS != 0
                && flags & (w::elevation::SCRIPT | w::elevation::PACKAGE) != 0
        {
            return Err(Code::InvalidRequest);
        }
        let whole = core::str::from_utf8(&b[10..]).map_err(|_| Code::InvalidRequest)?;
        // Windows Installer names its package after its own path; the
        // consent token binds the package and the binding of its hash and
        // the command line (`w::elevation::PACKAGE`), and only msiexec.exe
        // may ask.
        let (path, package, token) = if flags & w::elevation::PACKAGE != 0 {
            let mut fields = whole.splitn(3, '\0');
            let (Some(program), Some(package), Some(binding)) =
                (fields.next(), fields.next(), fields.next())
            else {
                return Err(Code::InvalidRequest);
            };
            if !w::elevation::MSIEXEC
                .iter()
                .any(|m| m.eq_ignore_ascii_case(program))
                || binding.len() != 64
                || !binding
                    .bytes()
                    .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
            {
                return Err(Code::InvalidRequest);
            }
            use sha2::Digest;
            let mut digest = sha2::Sha256::new();
            digest.update(w::elevation::CONSENT_TAG);
            digest.update(package.as_bytes());
            digest.update([0]);
            digest.update(binding.as_bytes());
            let token: alloc::string::String = digest
                .finalize()
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect();
            (program, Some(package), Some(token))
        } else if flags & w::elevation::ARGUMENTS != 0 {
            // `program NUL arguments` (each NUL-terminated; an empty
            // argument is a lone NUL): the elevated program gets them after
            // its own name.
            let (program, arguments) = whole.split_once('\0').ok_or(Code::InvalidRequest)?;
            if !arguments.ends_with('\0') || arguments.chars().any(|c| c.is_control() && c != '\0')
            {
                return Err(Code::InvalidRequest);
            }
            (program, None, Some(alloc::string::String::from(arguments)))
        } else {
            (whole, None, None)
        };
        // Native paths, or Windows paths on C: (shell32's runas).
        for p in core::iter::once(path).chain(package) {
            if !door_service_protocol::application::rooted(p) || p.chars().any(char::is_control) {
                return Err(Code::InvalidRequest);
            }
        }
        if self.shell.uac.is_some()
            || self.shell.elevated_launch.is_some()
            || self.session.is_none()
            || self.shell.locked
        {
            return Err(Code::WouldBlock);
        }
        let downloaded = flags & w::elevation::DOWNLOADED != 0;
        if flags & w::elevation::SCRIPT != 0 {
            if !t.is_empty() || image_bytes != 0 || !door_window::elevation::is_script(path) {
                return Err(Code::InvalidRequest);
            }
            self.ask_consent(door_window::elevation::script_consent(path, downloaded));
            return Ok(Response::empty());
        }
        // [working folder, image section]: the section is read-only and
        // holds exactly the bytes the caller says.
        if t.len() != 2 || image_bytes == 0 || image_bytes > EXECUTABLE_MAX_BYTES {
            return Err(Code::InvalidRequest);
        }
        let folder = door_sys::inspect(t[0].handle).map_err(server::status)?;
        let section = door_sys::inspect(t[1].handle).map_err(server::status)?;
        if folder[0] != HandleKind::Directory as u64
            || section[0] != HandleKind::Section as u64
            || section[1] & rights::WRITE != 0
        {
            return Err(Code::Denied);
        }
        // Snapshots copy at most SECTION_COPY_BYTES each.
        let mut bytes = alloc::vec![0; image_bytes];
        for (i, chunk) in bytes
            .chunks_mut(door_abi::graphics::SECTION_COPY_BYTES)
            .enumerate()
        {
            let at = (i * door_abi::graphics::SECTION_COPY_BYTES) as u64;
            if door_sys::section_snapshot(t[1].handle, at, chunk).map_err(server::status)?
                != chunk.len()
            {
                return Err(Code::InvalidRequest);
            }
        }
        let now = unix_time().ok_or(Code::Unsupported)?;
        // The shipped roots only: storage serves session peers and Init,
        // not this server, so the machine Root store is not read here.
        let mut inspection = door_window::elevation::inspect(
            path,
            &bytes,
            now,
            &door_authenticode::roots::trusted(),
        );
        if let Some(package) = package {
            inspection.program =
                alloc::string::String::from(package.rsplit(['\\', '/']).next().unwrap_or(package));
        }
        let _ = door_sys::write(
            format!(
                "DOOR_P16 AUTHENTICODE path={path} trust=0x{:08X} publisher={}\n",
                inspection.trust,
                inspection.publisher.as_deref().unwrap_or("Unknown")
            )
            .as_bytes(),
        );
        let directory = door_sys::duplicate(
            t[0].handle,
            rights::READ | rights::QUERY | rights::DUPLICATE | rights::TRANSFER,
        )
        .map_err(server::status)?;
        let mut consent =
            door_window::elevation::program_consent(path, &inspection, directory, downloaded);
        // A package shows the Windows Installer package's icon, not
        // msiexec.exe's; the elevated msiexec.exe gets the consent token.
        if let Some(package) = package {
            consent.icon =
                door_window::uac::Icon::File(door_window::apps::FileIcon::for_name(package, false));
            consent.image = None;
        }
        // The elevated program's arguments: Windows Installer's consent
        // token, or the caller's own.
        if let (Some(extra), door_window::uac::Then::Program { arguments, .. }) =
            (token, &mut consent.then)
        {
            *arguments = if package.is_some() {
                format!("{}{extra}\0", w::elevation::CONSENT_SWITCH)
            } else {
                extra
            };
        }
        // What Init will be asked to start must fit its request: refused
        // now, not after the user approved it.
        if let door_window::uac::Then::Program {
            path,
            cwd,
            image,
            arguments,
            ..
        } = &consent.then
            && door_service_client::Init::execute_elevated_body(image, path, cwd, arguments)
                .is_err()
        {
            let _ = door_sys::close(directory);
            return Err(Code::InvalidRequest);
        }
        self.ask_consent(consent);
        Ok(Response::empty())
    }
}
