//! The WSA* extended calls over the same sockets. With an OVERLAPPED they
//! queue the operation (crate::overlapped) and fail with WSA_IO_PENDING;
//! its event, completion port and completion routine report the end.
use crate::error::*;
use crate::guest::*;
use crate::overlapped;
use alloc::vec::Vec;

/// WSABUF: len u32, then buf (16 bytes on x64, 8 on x86).
fn buffers(pointer: u64, count: u32) -> Result<Vec<(u64, u32)>, u32> {
    use door_windows_abi::layout::{self, Field::*};
    const WSABUF: [layout::Field; 2] = [U32, Pointer];
    const BYTES: usize = layout::size(&WSABUF);
    if count == 0 || count > 1024 {
        return Err(WSAEINVAL);
    }
    let mut raw = alloc::vec![0u8; count as usize * BYTES];
    read(pointer, &mut raw)?;
    Ok(raw
        .chunks_exact(BYTES)
        .map(|b| {
            (
                layout::get(b, &WSABUF, 1),
                layout::get(b, &WSABUF, 0) as u32,
            )
        })
        .collect())
}
/// The SOCKET_ERROR/WSA_IO_PENDING result of a queued operation.
fn queued(result: Result<(), u32>) -> i32 {
    fail(result.err().unwrap_or(WSA_IO_PENDING))
}
#[unsafe(no_mangle)]
pub extern "system" fn WSASend(
    socket: usize,
    data: usize,
    count: u32,
    sent: usize,
    flags: u32,
    overlapped: usize,
    routine: usize,
) -> i32 {
    let socket = door_windows_abi::widen(socket);
    let data = door_windows_abi::widen(data);
    let sent = door_windows_abi::widen(sent);
    let overlapped = door_windows_abi::widen(overlapped);
    let routine = door_windows_abi::widen(routine);
    {
        if overlapped != 0 {
            if flags != 0 {
                return fail(WSAEOPNOTSUPP);
            }
            return queued(
                buffers(data, count).and_then(|b| overlapped::send(socket, b, overlapped, routine)),
            );
        }
        let result = (|| {
            if flags != 0 {
                return Err(WSAEOPNOTSUPP);
            }
            let mut total = 0u32;
            for (buffer, length) in buffers(data, count)? {
                let n = send_bytes(socket, buffer, length)?;
                total += n;
                if n < length {
                    break;
                }
            }
            if sent != 0 {
                write(sent, &total.to_le_bytes())?;
            }
            Ok(())
        })();
        result.map_or_else(fail, |()| ok())
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn WSASendTo(
    socket: usize,
    data: usize,
    count: u32,
    sent: usize,
    flags: u32,
    to: usize,
    to_length: i32,
    overlapped: usize,
    routine: usize,
) -> i32 {
    let socket = door_windows_abi::widen(socket);
    let data = door_windows_abi::widen(data);
    let sent = door_windows_abi::widen(sent);
    let to = door_windows_abi::widen(to);
    let overlapped = door_windows_abi::widen(overlapped);
    let routine = door_windows_abi::widen(routine);
    {
        if overlapped != 0 {
            return queued((|| {
                if flags != 0 {
                    return Err(WSAEOPNOTSUPP);
                }
                let list = buffers(data, count)?;
                let (address, port) = if to == 0 {
                    entry(socket)?.peer.ok_or(WSAENOTCONN)?
                } else {
                    sockaddr(to, to_length, entry(socket)?.family)?
                };
                if entry(socket)?.stream {
                    return Err(WSAEISCONN);
                }
                overlapped::send_to(socket, list, address, port, overlapped, routine)
            })());
        }
        let result = (|| {
            if flags != 0 {
                return Err(WSAEOPNOTSUPP);
            }
            let list = buffers(data, count)?;
            // One datagram: gather the buffers.
            let mut gathered = Vec::new();
            for (buffer, length) in &list {
                let start = gathered.len();
                gathered.resize(start + *length as usize, 0);
                read(*buffer, &mut gathered[start..])?;
            }
            let n = if to == 0 {
                let peer = entry(socket)?.peer.ok_or(WSAENOTCONN)?;
                send_datagram(
                    socket,
                    peer.0,
                    peer.1,
                    gathered.as_ptr() as u64,
                    gathered.len() as u32,
                )?
            } else {
                let (address, port) = sockaddr(to, to_length, entry(socket)?.family)?;
                send_datagram(
                    socket,
                    address,
                    port,
                    gathered.as_ptr() as u64,
                    gathered.len() as u32,
                )?
            };
            if sent != 0 {
                write(sent, &n.to_le_bytes())?;
            }
            Ok(())
        })();
        result.map_or_else(fail, |()| ok())
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn WSARecv(
    socket: usize,
    data: usize,
    count: u32,
    received: usize,
    flags: usize,
    overlapped: usize,
    routine: usize,
) -> i32 {
    let socket = door_windows_abi::widen(socket);
    let data = door_windows_abi::widen(data);
    let received = door_windows_abi::widen(received);
    let flags = door_windows_abi::widen(flags);
    let overlapped = door_windows_abi::widen(overlapped);
    let routine = door_windows_abi::widen(routine);
    {
        if overlapped != 0 {
            return queued((|| {
                if flags != 0 && read_i32(flags)? != 0 {
                    // MSG_PEEK, MSG_WAITALL and MSG_PARTIAL with an OVERLAPPED.
                    return Err(WSAEOPNOTSUPP);
                }
                overlapped::receive(socket, buffers(data, count)?, flags, overlapped, routine)
            })());
        }
        let result = (|| {
            let mode = if flags == 0 { 0 } else { read_i32(flags)? };
            let mut total = 0u32;
            for (i, (buffer, length)) in buffers(data, count)?.into_iter().enumerate() {
                // The first buffer waits as recv does; later ones take only what
                // is already queued, so a blocking socket never waits twice.
                if i > 0 && net_state(entry(socket)?.id)?.queued_in == 0 {
                    break;
                }
                let n = match receive_bytes(socket, buffer, length, mode) {
                    Ok(n) => n,
                    Err(WSAEWOULDBLOCK) if i > 0 => break,
                    Err(e) => return Err(e),
                };
                total += n;
                if n < length {
                    break;
                }
            }
            if received != 0 {
                write(received, &total.to_le_bytes())?;
            }
            if flags != 0 {
                write_i32(flags, 0)?;
            }
            Ok(())
        })();
        result.map_or_else(fail, |()| ok())
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn WSARecvFrom(
    socket: usize,
    data: usize,
    count: u32,
    received: usize,
    flags: usize,
    from: usize,
    from_length: usize,
    overlapped: usize,
    routine: usize,
) -> i32 {
    let socket = door_windows_abi::widen(socket);
    let data = door_windows_abi::widen(data);
    let received = door_windows_abi::widen(received);
    let flags = door_windows_abi::widen(flags);
    let from = door_windows_abi::widen(from);
    let from_length = door_windows_abi::widen(from_length);
    let overlapped = door_windows_abi::widen(overlapped);
    let routine = door_windows_abi::widen(routine);
    {
        if overlapped != 0 {
            return queued((|| {
                let list = buffers(data, count)?;
                if from != 0 && (from_length == 0 || read_i32(from_length)? < 16) {
                    return Err(WSAEFAULT);
                }
                overlapped::receive_from(
                    socket,
                    list[0],
                    from,
                    from_length,
                    flags,
                    overlapped,
                    routine,
                )
            })());
        }
        let result = (|| {
            let list = buffers(data, count)?;
            let (buffer, length) = list[0];
            let (n, _, _) = receive_datagram(socket, buffer, length, from, from_length)?;
            if received != 0 {
                write(received, &n.to_le_bytes())?;
            }
            if flags != 0 {
                write_i32(flags, 0)?;
            }
            Ok(())
        })();
        result.map_or_else(fail, |()| ok())
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn WSAConnect(
    socket: usize,
    name: usize,
    length: i32,
    caller: usize,
    callee: usize,
    sqos: usize,
    gqos: usize,
) -> i32 {
    let socket = door_windows_abi::widen(socket);
    let name = door_windows_abi::widen(name);
    let caller = door_windows_abi::widen(caller);
    let callee = door_windows_abi::widen(callee);
    let sqos = door_windows_abi::widen(sqos);
    let gqos = door_windows_abi::widen(gqos);
    {
        if caller != 0 || callee != 0 || gqos != 0 {
            return fail(WSAEOPNOTSUPP);
        }
        let _ = sqos;
        connect(
            door_windows_abi::narrow(socket),
            door_windows_abi::narrow(name),
            length,
        )
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn WSAAccept(
    socket: usize,
    address: usize,
    length: usize,
    condition: usize,
    _data: usize,
) -> usize {
    let socket = door_windows_abi::widen(socket);
    let address = door_windows_abi::widen(address);
    let length = door_windows_abi::widen(length);
    let condition = door_windows_abi::widen(condition);
    door_windows_abi::narrow((move || -> u64 {
        if condition != 0 {
            fail(WSAEOPNOTSUPP);
            return INVALID_SOCKET;
        }
        door_windows_abi::widen(accept(
            door_windows_abi::narrow(socket),
            door_windows_abi::narrow(address),
            door_windows_abi::narrow(length),
        ))
    })())
}
const FIONBIO: u32 = 0x8004_667E;
const FIONREAD: u32 = 0x4004_667F;
const SIO_KEEPALIVE_VALS: u32 = 0x9800_0004;
#[unsafe(no_mangle)]
pub extern "system" fn WSAIoctl(
    socket: usize,
    code: u32,
    input: usize,
    input_length: u32,
    output: usize,
    output_length: u32,
    returned: usize,
    overlapped: usize,
    routine: usize,
) -> i32 {
    let socket = door_windows_abi::widen(socket);
    let input = door_windows_abi::widen(input);
    let output = door_windows_abi::widen(output);
    let returned = door_windows_abi::widen(returned);
    let overlapped = door_windows_abi::widen(overlapped);
    let routine = door_windows_abi::widen(routine);
    {
        let result = (|| -> Result<u32, u32> {
            match code {
                FIONBIO | FIONREAD => {
                    let pointer = if code == FIONBIO { input } else { output };
                    let size = if code == FIONBIO {
                        input_length
                    } else {
                        output_length
                    };
                    if size < 4 {
                        return Err(WSAEFAULT);
                    }
                    match ioctlsocket(
                        door_windows_abi::narrow(socket),
                        code as i32,
                        door_windows_abi::narrow(pointer),
                    ) {
                        0 => Ok(if code == FIONREAD { 4 } else { 0 }),
                        _ => Err(WSAGetLastError() as u32),
                    }
                }
                // tcp_keepalive { onoff, keepalivetime, keepaliveinterval } (ms).
                SIO_KEEPALIVE_VALS => {
                    if input_length < 12 {
                        return Err(WSAEFAULT);
                    }
                    let mut b = [0u8; 12];
                    read(input, &mut b)?;
                    let on = u32::from_le_bytes(b[..4].try_into().unwrap()) != 0;
                    let time = u32::from_le_bytes(b[4..8].try_into().unwrap());
                    let e = entry(socket)?;
                    unsafe { DoorNetOption(e.id, 2, if on { time.max(1) } else { 0 }) };
                    with_entry(socket, |e| {
                        e.keep_alive = on;
                        Ok(0)
                    })
                }
                // The Microsoft extensions (AcceptEx, ConnectEx...) by GUID.
                SIO_GET_EXTENSION_FUNCTION_POINTER => {
                    entry(socket)?;
                    if input_length < 16 || (output_length as usize) < POINTER {
                        return Err(WSAEFAULT);
                    }
                    let mut id = [0u8; 16];
                    read(input, &mut id)?;
                    let function = overlapped::extension(&id).ok_or(WSAEINVAL)?;
                    write_pointer(output, function)?;
                    Ok(POINTER as u32)
                }
                // No layered providers: the base handle is the socket.
                SIO_BASE_HANDLE | SIO_BSP_HANDLE | SIO_BSP_HANDLE_SELECT | SIO_BSP_HANDLE_POLL => {
                    entry(socket)?;
                    if (output_length as usize) < POINTER {
                        return Err(WSAEFAULT);
                    }
                    write_pointer(output, socket)?;
                    Ok(POINTER as u32)
                }
                _ => Err(WSAEOPNOTSUPP),
            }
        })();
        match result {
            Ok(n) => {
                if returned != 0 {
                    let _ = write(returned, &n.to_le_bytes());
                }
                // These finish at once; an OVERLAPPED still hears of it.
                if overlapped != 0 {
                    overlapped::finished(socket, overlapped, routine, n);
                }
                ok()
            }
            Err(e) => fail(e),
        }
    }
}
const SIO_GET_EXTENSION_FUNCTION_POINTER: u32 = 0xC800_0006;
const SIO_BASE_HANDLE: u32 = 0x4800_0022;
const SIO_BSP_HANDLE: u32 = 0x4800_001B;
const SIO_BSP_HANDLE_SELECT: u32 = 0x4800_001C;
const SIO_BSP_HANDLE_POLL: u32 = 0x4800_001D;
#[unsafe(no_mangle)]
pub extern "system" fn WSAGetOverlappedResult(
    _socket: usize,
    overlapped: usize,
    transferred: usize,
    wait: i32,
    flags: usize,
) -> i32 {
    let overlapped = door_windows_abi::widen(overlapped);
    let transferred = door_windows_abi::widen(transferred);
    let flags = door_windows_abi::widen(flags);
    {
        if overlapped == 0 || transferred == 0 {
            fail(WSAEFAULT);
            return 0;
        }
        match overlapped::result(overlapped, transferred, wait != 0, flags) {
            Ok(()) => 1,
            Err(e) => {
                fail(e);
                0
            }
        }
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn WSAIsBlocking() -> i32 {
    0
}
