//! Producer mapping: private drawing bytes are copied into the shared section.
//! No Rust slice is ever formed over memory simultaneously visible to the server.
use alloc::{format, vec::Vec};
use door_abi::{Handle, Status, rights};
use door_service_client::{Error, window::WindowClient};
const ADDRESS: u64 = 0x7200_0000;
const GRANULE: u64 = 4 * 1024 * 1024;
/// 48 x 4 MiB = 0x7200_0000..0x7E00_0000; the top 32 MiB of the user section
/// window holds the read-only application asset pack (`assets::ADDRESS`).
const GRANULES: usize = 48;
const _: () = assert!(ADDRESS + GRANULES as u64 * GRANULE == crate::assets::ADDRESS);
static MAPPINGS: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);
/// Free heap that must remain beside a newly admitted private frame (icons,
/// decoding and document operations).
pub const FRAME_MARGIN: usize = 24 * 1024 * 1024;
fn resources() -> Error {
    Error::Service(door_service_protocol::Code::Resources)
}
/// A zeroed private frame of `bytes`, admitted only when the working margin
/// would still be free beside it.
pub fn admit_frame(bytes: usize) -> Result<Vec<u8>, Error> {
    let mut margin = Vec::<u8>::new();
    margin
        .try_reserve_exact(FRAME_MARGIN)
        .map_err(|_| resources())?;
    let mut pixels = Vec::new();
    pixels.try_reserve_exact(bytes).map_err(|_| resources())?;
    pixels.resize(bytes, 0);
    drop(margin);
    Ok(pixels)
}
/// A zeroed frame of `bytes` in space just released (no margin check).
/// Empty when even that fails.
fn restore_frame(bytes: usize) -> Vec<u8> {
    let mut pixels = Vec::new();
    if pixels.try_reserve_exact(bytes).is_ok() {
        pixels.resize(bytes, 0);
    }
    pixels
}
/// Replace a private frame by an admitted one of `bytes`. The old frame is
/// released first: every dirty frame repaints the whole window, so its
/// contents are never needed, and growth then needs the new frame plus the
/// margin rather than both frames plus the margin (at 3x a maximized window
/// is 31 MB beside a 25 MB restored-size frame). When admission is refused,
/// a frame of the old length is rebuilt in the released space and returned
/// with the error (empty only if even that fails).
pub fn grow_frame(frame: Vec<u8>, bytes: usize) -> Result<Vec<u8>, (Vec<u8>, Error)> {
    let retained = frame.len();
    drop(frame);
    admit_frame(bytes).map_err(|error| (restore_frame(retained), error))
}
/// A new section holding a copy of `bytes` (from offset 0) for a service
/// that snapshots it, e.g. a staged save. It is mapped only while being
/// filled, in a free surface granule; the caller owns and closes the handle.
pub(crate) fn upload_section(bytes: &[u8]) -> Result<Handle, Status> {
    use core::sync::atomic::Ordering;
    let mapped = (bytes.len() as u64).div_ceil(4096).max(1) * 4096;
    let section = door_sys::section_create(mapped, rights::ALL & !rights::WAIT)?;
    let (address, reservation) = match reserve(&MAPPINGS, mapped) {
        Ok(v) => v,
        Err(e) => {
            let _ = door_sys::close(section);
            return Err(e);
        }
    };
    // SAFETY: this live reservation exclusively owns the range; the copy
    // finishes before the mapping is removed and before any service sees
    // the handle, so no Rust slice aliases shared memory.
    let filled = unsafe { door_sys::section_map(section, address, rights::READ | rights::WRITE) }
        .map(|_| unsafe {
            core::ptr::copy_nonoverlapping(bytes.as_ptr(), address as *mut u8, bytes.len());
        });
    // An unmap failure keeps the granule reserved rather than reuse a range
    // that may still be mapped.
    if filled.is_err() || unsafe { door_sys::section_unmap(address, mapped) }.is_ok() {
        MAPPINGS.fetch_and(!reservation, Ordering::Release);
    }
    if let Err(e) = filled {
        let _ = door_sys::close(section);
        return Err(e);
    }
    Ok(section)
}
fn reserve(bits: &core::sync::atomic::AtomicU64, bytes: u64) -> Result<(u64, u64), Status> {
    use core::sync::atomic::Ordering;
    let count = bytes.div_ceil(GRANULE) as usize;
    if count == 0 || count > GRANULES {
        return Err(Status::InvalidArgument);
    }
    loop {
        let old = bits.load(Ordering::Acquire);
        let mask = (0..=GRANULES - count)
            .map(|i| (((1u64 << count) - 1) << i, i))
            .find(|(m, _)| old & m == 0)
            .ok_or(Status::ResourceExhausted)?;
        if bits
            .compare_exchange_weak(old, old | mask.0, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
        {
            return Ok((ADDRESS + mask.1 as u64 * GRANULE, mask.0));
        }
    }
}
pub struct Surface {
    pub pixels: Vec<u8>,
    pub width: u32,
    pub height: u32,
    section: Handle,
    address: u64,
    reservation: u64,
    pub buffer: u64,
    mapped: u64,
    pub available: bool,
    attached: bool,
    /// The section holds exactly the last frame the server accepted from this
    /// buffer, so only rows that differ need to be written and committed.
    synced: bool,
    /// Rows written for the pending attachment (`None`: the whole frame).
    band: Option<(u32, u32)>,
    /// One row read back from the section for exact comparison.
    scratch: Vec<u8>,
}
impl Surface {
    /// CLOSE cancels an uncommitted server attachment without transferring
    /// producer ownership or emitting a release. Preserve availability of any
    /// already published buffer; only retire the local pending-attachment hint.
    pub fn close_requested(&mut self) {
        if self.attached {
            self.attached = false;
            let _ = door_sys::write(b"DOOR_P12 SURFACE ATTACHMENT_RETIRED\n");
        }
        // Rows written for the retired attachment were never accepted.
        self.synced = false;
    }
    /// First and one-past-last rows whose private pixels differ from the
    /// section. Rows are read with a raw copy (no slice over shared memory);
    /// the server only reads the section, and never while we own it.
    fn changed_rows(&mut self) -> Option<(usize, usize)> {
        let stride = self.width as usize * 4;
        let rows = self.height as usize;
        self.scratch.resize(stride, 0);
        let mut differs = |row: usize| {
            // SAFETY: `row < height` and the mapping covers height * stride bytes.
            unsafe {
                core::ptr::copy_nonoverlapping(
                    (self.address as *const u8).add(row * stride),
                    self.scratch.as_mut_ptr(),
                    stride,
                );
            }
            self.scratch[..] != self.pixels[row * stride..(row + 1) * stride]
        };
        let first = (0..rows).find(|row| differs(*row))?;
        let last = (first..rows)
            .rev()
            .find(|row| differs(*row))
            .unwrap_or(first);
        Some((first, last + 1))
    }
    /// Shrinking/restoring reuses the owned mapping and backing allocation.
    /// Register a new descriptor only after the old attachment is retired.
    pub fn reconfigure(
        &mut self,
        client: &mut WindowClient,
        width: u32,
        height: u32,
    ) -> Result<(), Error> {
        let bytes = u64::from(width)
            .checked_mul(u64::from(height))
            .and_then(|n| n.checked_mul(4))
            .filter(|n| *n > 0 && *n <= door_abi::graphics::SECTION_MAX_BYTES)
            .ok_or(Status::InvalidArgument)?;
        let retained = self.pixels.len();
        if bytes > self.mapped {
            // Release the private frame before admitting the larger one, so
            // growth never needs both frames plus the working margin at once.
            let frame = core::mem::take(&mut self.pixels);
            let pixels = match grow_frame(frame, bytes as usize) {
                Ok(pixels) => pixels,
                Err((old, error)) => return Err(self.refused(old, retained, bytes, error)),
            };
            return match Self::with_frame(client, width, height, pixels) {
                Ok(next) => {
                    let old = core::mem::replace(self, next);
                    old.unregister(client);
                    Ok(())
                }
                Err((pixels, error)) => {
                    drop(pixels);
                    Err(self.refused(restore_frame(retained), retained, bytes, error))
                }
            };
        }
        if bytes as usize > self.pixels.capacity() {
            let frame = core::mem::take(&mut self.pixels);
            match grow_frame(frame, bytes as usize) {
                Ok(mut pixels) => {
                    // Keep the current length until the new size is registered.
                    pixels.truncate(retained);
                    self.pixels = pixels;
                }
                Err((old, error)) => return Err(self.refused(old, retained, bytes, error)),
            }
        }
        let next = client.register(self.section, width, height, u64::from(width) * 4, bytes)?;
        if let Err(e) = client.unregister(self.buffer) {
            let _ = client.unregister(next);
            return Err(e);
        }
        self.buffer = next;
        self.pixels.resize(bytes as usize, 0);
        self.width = width;
        self.height = height;
        self.available = true;
        self.attached = false;
        // New descriptor: the section layout no longer matches any snapshot.
        self.synced = false;
        Ok(())
    }
    /// A refused resize keeps the current surface: `old` is the frame restored
    /// at its previous length. Reports the refusal (never silent); a frame
    /// that could not even be restored is fatal for this connection.
    fn refused(&mut self, old: Vec<u8>, retained: usize, requested: u64, error: Error) -> Error {
        let restored = old.len() == retained;
        self.pixels = old;
        self.synced = false;
        let _ = door_sys::write(
            format!(
                "DOOR_P12 SURFACE RESIZE REFUSED requested={requested} retained={retained} restored={} error={error:?}\n",
                u8::from(restored)
            )
            .as_bytes(),
        );
        if restored {
            error
        } else {
            Error::Service(door_service_protocol::Code::Io)
        }
    }
    pub fn new(client: &mut WindowClient, width: u32, height: u32) -> Result<Self, Error> {
        let bytes = u64::from(width)
            .checked_mul(u64::from(height))
            .and_then(|n| n.checked_mul(4))
            .ok_or(Status::InvalidArgument)?;
        if bytes == 0 || bytes > door_abi::graphics::SECTION_MAX_BYTES {
            return Err(Status::InvalidArgument.into());
        }
        // Fallible admission happens before publishing any producer mapping.
        let pixels = admit_frame(bytes as usize).inspect_err(|error| {
            let _ = door_sys::write(
                format!("DOOR_P12 SURFACE RESOURCES requested={bytes} error={error:?}\n")
                    .as_bytes(),
            );
        })?;
        Self::with_frame(client, width, height, pixels).map_err(|(_, error)| error)
    }
    /// Publish a surface for an admitted private frame of exactly
    /// `width * height * 4` bytes; on failure the frame is handed back.
    fn with_frame(
        client: &mut WindowClient,
        width: u32,
        height: u32,
        pixels: Vec<u8>,
    ) -> Result<Self, (Vec<u8>, Error)> {
        let bytes = pixels.len() as u64;
        let mapped = (bytes + 4095) & !4095;
        let section = match door_sys::section_create(mapped, rights::ALL & !rights::WAIT)
            .inspect_err(|error| {
                let _ = door_sys::write(
                    format!("DOOR_P12 SURFACE CREATE bytes={mapped} error={error:?}\n").as_bytes(),
                );
            }) {
            Ok(section) => section,
            Err(e) => return Err((pixels, e.into())),
        };
        let (address, reservation) = match reserve(&MAPPINGS, mapped) {
            Ok(v) => v,
            Err(e) => {
                let _ = door_sys::close(section);
                return Err((pixels, e.into()));
            }
        };
        // SAFETY: this live reservation exclusively owns the complete producer range.
        if let Err(e) =
            unsafe { door_sys::section_map(section, address, rights::READ | rights::WRITE) }
        {
            let _ = door_sys::write(
                format!("DOOR_P12 SURFACE MAP bytes={mapped} error={e:?}\n").as_bytes(),
            );
            MAPPINGS.fetch_and(!reservation, core::sync::atomic::Ordering::Release);
            let _ = door_sys::close(section);
            return Err((pixels, e.into()));
        }
        let buffer = match client.register(section, width, height, u64::from(width) * 4, bytes) {
            Ok(id) => id,
            Err(e) => {
                let _ = door_sys::write(
                    format!("DOOR_P12 SURFACE REGISTER bytes={mapped} error={e:?}\n").as_bytes(),
                );
                if unsafe { door_sys::section_unmap(address, mapped) }.is_ok() {
                    MAPPINGS.fetch_and(!reservation, core::sync::atomic::Ordering::Release);
                }
                let _ = door_sys::close(section);
                return Err((pixels, e));
            }
        };
        Ok(Self {
            pixels,
            width,
            height,
            section,
            address,
            reservation,
            buffer,
            mapped,
            available: true,
            attached: false,
            synced: false,
            band: None,
            scratch: Vec::new(),
        })
    }
    pub fn present(
        &mut self,
        client: &mut WindowClient,
        window: u64,
        epoch: u64,
    ) -> Result<bool, Error> {
        if !self.available {
            return Err(Status::WouldBlock.into());
        }
        let fresh = !self.attached;
        if fresh {
            let stride = self.width as usize * 4;
            let rows = self.height as usize;
            // With a synced section only the changed rows move; an unchanged
            // frame still commits one row so the server releases the buffer.
            self.band = (self.synced && client.supports_band_commit()).then(|| {
                let (top, end) = self.changed_rows().unwrap_or((0, 1));
                (top as u32, (end - top) as u32)
            });
            let (top, count) = self
                .band
                .map_or((0, rows), |(t, c)| (t as usize, c as usize));
            // SAFETY: the buffer is available (the server is not reading it)
            // and the mapping covers height * stride bytes; raw copies form no
            // Rust reference over the shared section.
            unsafe {
                core::ptr::copy_nonoverlapping(
                    self.pixels.as_ptr().add(top * stride),
                    (self.address as *mut u8).add(top * stride),
                    count * stride,
                );
            }
            self.synced = false;
            client.attach(window, self.buffer, epoch)?;
            self.attached = true;
        }
        let committed = match self.band {
            Some((top, rows)) => client.commit_band(window, epoch, top, rows),
            None => client.commit_epoch(window, epoch),
        };
        if let Err(error) = committed {
            if matches!(
                error,
                Error::Service(
                    door_service_protocol::Code::StaleInstance
                        | door_service_protocol::Code::Closing
                )
            ) {
                self.close_requested();
            }
            return Err(error);
        }
        self.attached = false;
        self.available = false;
        self.synced = true;
        Ok(fresh)
    }
    pub fn unregister(self, client: &mut WindowClient) {
        if let Err(error) = client.unregister(self.buffer) {
            let _ = door_sys::write(
                format!(
                    "DOOR_P12 SURFACE UNREGISTER buffer={} error={error:?}\n",
                    self.buffer
                )
                .as_bytes(),
            );
        }
    }
}
impl Drop for Surface {
    fn drop(&mut self) {
        unsafe {
            if let Err(error) = door_sys::section_unmap(self.address, self.mapped) {
                let _ = door_sys::write(
                    format!(
                        "DOOR_P12 SURFACE UNMAP bytes={} error={error:?}\n",
                        self.mapped
                    )
                    .as_bytes(),
                );
            } else {
                MAPPINGS.fetch_and(!self.reservation, core::sync::atomic::Ordering::Release);
            }
        }
        let _ = door_sys::close(self.section);
    }
}

#[cfg(test)]
mod mapping_tests {
    use super::*;
    #[test]
    fn simultaneous_surfaces_never_alias_and_freed_holes_are_reused() {
        let bits = core::sync::atomic::AtomicU64::new(0);
        let (a, ma) = reserve(&bits, 33 * 1024 * 1024).unwrap();
        let (b, mb) = reserve(&bits, 5 * 1024 * 1024).unwrap();
        assert_ne!(a, b);
        assert_eq!(ma & mb, 0);
        bits.fetch_and(!ma, core::sync::atomic::Ordering::Release);
        let (c, mc) = reserve(&bits, 33 * 1024 * 1024).unwrap();
        assert_eq!((a, ma), (c, mc));
        assert_eq!(mc & mb, 0);
        assert!(reserve(&bits, 224 * 1024 * 1024).is_err());
    }
}
