use super::*;
use door_service_protocol::{BODY_BYTES, application};

/// Init mediates file admission; clients do not receive a Launch capability.
pub struct Applications {
    connection: Connection,
}
impl Applications {
    pub fn connect(registry: Handle) -> Result<Self> {
        Ok(Self {
            connection: Connection::connect(registry, manifest::INIT)?,
        })
    }
    pub fn execute(
        &mut self,
        path: &str,
        cwd: &str,
        arguments: &[u8],
        directory: Handle,
    ) -> Result<Handle> {
        self.launch(
            application::EXECUTE,
            path,
            cwd,
            arguments,
            directory,
            None,
            false,
            [0; 3],
            0,
        )
    }
    /// `execute` in a Terminal tab's console (`console::Command::Open`):
    /// the program and its children use that console.
    pub fn execute_in_console(
        &mut self,
        path: &str,
        cwd: &str,
        arguments: &[u8],
        directory: Handle,
        console: u8,
    ) -> Result<Handle> {
        self.launch(
            application::EXECUTE,
            path,
            cwd,
            arguments,
            directory,
            None,
            false,
            [0; 3],
            console,
        )
    }
    /// CreateProcess with an environment block (NUL-separated `NAME=value`,
    /// replacing the session's) and/or CREATE_SUSPENDED.
    #[allow(clippy::too_many_arguments)]
    pub fn execute_with(
        &mut self,
        detached: bool,
        path: &str,
        cwd: &str,
        arguments: &[u8],
        directory: Handle,
        environment: Option<&[u8]>,
        suspended: bool,
    ) -> Result<Handle> {
        self.execute_piped(
            detached,
            path,
            cwd,
            arguments,
            directory,
            environment,
            suspended,
            [0; 3],
        )
    }
    /// `execute_with`, and the pipe sections that become the child's
    /// standard input, output and error (0: the console).
    #[allow(clippy::too_many_arguments)]
    pub fn execute_piped(
        &mut self,
        detached: bool,
        path: &str,
        cwd: &str,
        arguments: &[u8],
        directory: Handle,
        environment: Option<&[u8]>,
        suspended: bool,
        standard: [Handle; 3],
    ) -> Result<Handle> {
        let operation = if detached {
            application::EXECUTE_DETACHED
        } else {
            application::EXECUTE
        };
        self.launch(
            operation,
            path,
            cwd,
            arguments,
            directory,
            environment,
            suspended,
            standard,
            0,
        )
    }
    /// A launch without a console (Explorer): Windows GUI images get NULL
    /// standard handles.
    pub fn execute_detached(
        &mut self,
        path: &str,
        cwd: &str,
        arguments: &[u8],
        directory: Handle,
    ) -> Result<Handle> {
        self.launch(
            application::EXECUTE_DETACHED,
            path,
            cwd,
            arguments,
            directory,
            None,
            false,
            [0; 3],
            0,
        )
    }
    #[allow(clippy::too_many_arguments)]
    fn launch(
        &mut self,
        operation: u32,
        path: &str,
        cwd: &str,
        arguments: &[u8],
        directory: Handle,
        environment: Option<&[u8]>,
        suspended: bool,
        standard: [Handle; 3],
        console: u8,
    ) -> Result<Handle> {
        let mut bytes = [0; BODY_BYTES];
        let invalid = || Error::Service(door_service_protocol::Code::InvalidRequest);
        let options = application::ExecuteOptions {
            suspended,
            environment: match environment {
                Some(_) => u16::try_from(arguments.len()).map_err(|_| invalid())?,
                None => 0,
            },
            standard: standard
                .iter()
                .enumerate()
                .filter(|(_, h)| **h != 0)
                .fold(0, |m, (i, _)| m | 1 << i),
            console,
        };
        // Arguments longer than the startup record holds (or than fit the
        // request), an environment block or a suspended start are staged
        // first; the child fetches them at startup.
        let staged = arguments.len() > door_abi::platform::ARGUMENT_BYTES
            || 6 + path.len() + cwd.len() + arguments.len() > BODY_BYTES
            || options != application::ExecuteOptions::NONE;
        let total = arguments.len() + environment.map_or(0, <[u8]>::len);
        if staged {
            if total > application::LONG_ARGUMENT_BYTES
                || (environment.is_some() && arguments.len() > application::COMMAND_LINE_BYTES)
            {
                return Err(invalid());
            }
            let block = arguments
                .iter()
                .chain(environment.unwrap_or(&[]).iter())
                .copied();
            let mut offset = 0;
            let mut part = [0u8; BODY_BYTES - 4];
            let mut filled = 0;
            let mut send = |part: &[u8], offset: &mut usize| -> Result<()> {
                let mut stage = [0; BODY_BYTES];
                stage[..2].copy_from_slice(&(*offset as u16).to_le_bytes());
                stage[2..4].copy_from_slice(&(total as u16).to_le_bytes());
                stage[4..4 + part.len()].copy_from_slice(part);
                self.connection
                    .call(application::ARGUMENTS_STAGE, &stage[..4 + part.len()], &[])?
                    .empty()?;
                *offset += part.len();
                Ok(())
            };
            for byte in block {
                part[filled] = byte;
                filled += 1;
                if filled == part.len() {
                    send(&part, &mut offset)?;
                    filled = 0;
                }
            }
            if filled != 0 {
                send(&part[..filled], &mut offset)?;
            }
        }
        let count = application::Execute {
            path,
            cwd,
            arguments: if staged { &[] } else { arguments },
            staged,
            options,
        }
        .encode(&mut bytes)
        .map_err(Error::Service)?;
        let mut transfers = [Transfer {
            handle: directory,
            rights: rights::READ | rights::QUERY | rights::DUPLICATE | rights::TRANSFER,
            mode: 0,
        }; 4];
        let mut sent = 1;
        for handle in standard.iter().filter(|h| **h != 0) {
            transfers[sent] = Transfer {
                handle: *handle,
                rights: rights::READ
                    | rights::WRITE
                    | rights::MAP
                    | rights::QUERY
                    | rights::DUPLICATE
                    | rights::TRANSFER,
                mode: 0,
            };
            sent += 1;
        }
        self.connection
            .call(operation, &bytes[..count], &transfers[..sent])?
            .one_handle()
    }
    /// This process's staged launch arguments (it was started with
    /// `application::LONG_ARGUMENTS_ENVIRONMENT`), into `out`: their length.
    pub fn staged_arguments(&mut self, out: &mut [u8]) -> Result<usize> {
        let mut read = 0;
        loop {
            let reply = self.connection.call(
                application::ARGUMENTS_FETCH,
                &(read as u16).to_le_bytes(),
                &[],
            )?;
            let data = reply.data()?;
            if data.len() < 4 {
                return Err(Error::Malformed);
            }
            let total = usize::from(u16::from_le_bytes([data[0], data[1]]));
            let part = &data[4..];
            if total > out.len() || read + part.len() > total {
                return Err(Error::Malformed);
            }
            out[read..read + part.len()].copy_from_slice(part);
            read += part.len();
            if read == total {
                return Ok(total);
            }
            if part.is_empty() {
                return Err(Error::Malformed);
            }
        }
    }
    /// The desktop shell's working-folder authority, for Init's scheduled
    /// tasks and services (once per sign-in).
    pub fn bind_directory(&mut self, directory: Handle) -> Result<()> {
        self.connection
            .call(
                application::SESSION_DIRECTORY,
                &[],
                &[Transfer {
                    handle: directory,
                    rights: rights::READ | rights::QUERY | rights::DUPLICATE | rights::TRANSFER,
                    mode: 0,
                }],
            )?
            .empty()
    }
    pub fn power(&mut self, reboot: bool) -> Result<()> {
        self.connection
            .call(application::POWER, &[u8::from(reboot)], &[])?
            .empty()
    }
    pub fn cancel_children(&mut self) -> Result<()> {
        self.connection
            .call(application::CANCEL_CHILDREN, &[], &[])?
            .empty()
    }
    /// Session Terminal only: end every program it started (or theirs)
    /// in the tab console `console`.
    pub fn cancel_console(&mut self, console: u8) -> Result<()> {
        self.connection
            .call(application::CANCEL_CHILDREN, &[console], &[])?
            .empty()
    }
}
