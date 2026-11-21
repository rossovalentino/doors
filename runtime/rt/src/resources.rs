//! On-demand read-only resources (`door_abi::platform::resource`, the full
//! Windows fonts): opened once per process, mapped read-only in the resource
//! view window and kept for the process lifetime, so lookups are `'static`.
//! The kernel shares one physical copy between every process that opens one.
use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use door_abi::{
    Status,
    platform::resource::{PATHS, VIEW_END, VIEW_START},
};

const COUNT: usize = PATHS.len();
/// Mapped view of resource `id` as (address, bytes); zero when not mapped.
static VIEWS: [(AtomicU64, AtomicU64); COUNT] =
    [const { (AtomicU64::new(0), AtomicU64::new(0)) }; COUNT];
static NEXT: AtomicU64 = AtomicU64::new(VIEW_START);
static LOCK: AtomicBool = AtomicBool::new(false);
/// View placement granularity (views start on 2 MiB boundaries).
const ALIGN: u64 = 2 * 1024 * 1024;

fn index(id: u64) -> Result<usize, Status> {
    usize::try_from(id)
        .ok()
        .and_then(|id| id.checked_sub(1))
        .filter(|i| *i < COUNT)
        .ok_or(Status::InvalidArgument)
}
fn mapped(i: usize) -> Option<&'static [u8]> {
    let bytes = VIEWS[i].1.load(Ordering::Acquire);
    (bytes != 0).then(|| {
        let address = VIEWS[i].0.load(Ordering::Acquire);
        // SAFETY: published only after a successful read-only mapping that is
        // never unmapped; the bytes are immutable for the process lifetime.
        unsafe { core::slice::from_raw_parts(address as *const u8, bytes as usize) }
    })
}
/// The bytes of resource `id`, mapping it on first use.
pub fn open(id: u64) -> Result<&'static [u8], Status> {
    let i = index(id)?;
    if let Some(bytes) = mapped(i) {
        return Ok(bytes);
    }
    while LOCK
        .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
        .is_err()
    {
        core::hint::spin_loop();
    }
    let result = (|| {
        if let Some(bytes) = mapped(i) {
            return Ok(bytes);
        }
        let handle = door_sys::resource_open(id)?;
        let address = NEXT.load(Ordering::Relaxed);
        // SAFETY: the resource window is reserved for these read-only views
        // and `address` lies past every earlier view; no reference exists yet.
        let mapped = unsafe { door_sys::section_map(handle, address, door_abi::rights::READ) };
        let _ = door_sys::close(handle);
        let bytes = mapped?;
        let end = address
            .checked_add(bytes.div_ceil(ALIGN) * ALIGN)
            .filter(|end| *end <= VIEW_END)
            .ok_or(Status::ResourceExhausted)?;
        NEXT.store(end, Ordering::Relaxed);
        VIEWS[i].0.store(address, Ordering::Release);
        VIEWS[i].1.store(bytes, Ordering::Release);
        mapped_or(i)
    })();
    LOCK.store(false, Ordering::Release);
    result
}
fn mapped_or(i: usize) -> Result<&'static [u8], Status> {
    mapped(i).ok_or(Status::InvalidArgument)
}
