//! Dynamic TLS values belong to the current native FS page, separately from GS.
use super::guest::RtlSetLastWin32Error;
use door_windows_abi::{error, tls};
static INDEXES: tls::Indexes = tls::Indexes::new();

#[unsafe(no_mangle)]
pub extern "system" fn DoorTlsAlloc() -> u32 {
    INDEXES.allocate().unwrap_or_else(|| {
        RtlSetLastWin32Error(error::NOT_ENOUGH_MEMORY);
        tls::OUT_OF_INDEXES
    })
}
#[unsafe(no_mangle)]
pub extern "system" fn DoorTlsFree(index: u32) -> i32 {
    if INDEXES.free(index) {
        1
    } else {
        RtlSetLastWin32Error(error::INVALID_PARAMETER);
        0
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn DoorTlsGetValue(index: u32) -> u64 {
    let Some(epoch) = INDEXES.epoch(index) else {
        RtlSetLastWin32Error(error::INVALID_PARAMETER);
        return 0;
    };
    let (tag, value) = super::arch::tls_slot(index);
    RtlSetLastWin32Error(0);
    if tag == epoch { value } else { 0 }
}
#[unsafe(no_mangle)]
pub extern "system" fn DoorTlsSetValue(index: u32, value: u64) -> i32 {
    let Some(epoch) = INDEXES.epoch(index) else {
        RtlSetLastWin32Error(error::INVALID_PARAMETER);
        return 0;
    };
    super::arch::set_tls_slot(index, epoch, value);
    1
}

// ---- Fiber-local storage -----------------------------------------------
// Doors has no fibers, so a thread's FLS is its TLS: an FLS index is a TLS
// index with an optional callback, run for the thread's non-null value
// when the thread exits and for the caller's value when the index is freed.

static CALLBACKS: [core::sync::atomic::AtomicU64; tls::SLOTS] =
    [const { core::sync::atomic::AtomicU64::new(0) }; tls::SLOTS];
#[unsafe(no_mangle)]
pub extern "system" fn DoorFlsAlloc(callback: u64) -> u32 {
    let index = DoorTlsAlloc();
    if index != tls::OUT_OF_INDEXES {
        CALLBACKS[index as usize].store(callback, core::sync::atomic::Ordering::Release);
    }
    index
}
fn run_callback(index: u32) {
    let callback = CALLBACKS[index as usize].load(core::sync::atomic::Ordering::Acquire);
    let value = DoorTlsGetValue(index);
    if callback != 0 && value != 0 {
        DoorTlsSetValue(index, 0);
        // PFLS_CALLBACK_FUNCTION: one pointer-sized argument.
        let function: extern "system" fn(usize) =
            unsafe { core::mem::transmute(callback as usize) };
        function(value as usize);
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn DoorFlsFree(index: u32) -> i32 {
    if INDEXES.epoch(index).is_none() {
        RtlSetLastWin32Error(error::INVALID_PARAMETER);
        return 0;
    }
    run_callback(index);
    CALLBACKS[index as usize].store(0, core::sync::atomic::Ordering::Release);
    DoorTlsFree(index)
}
#[unsafe(no_mangle)]
pub extern "system" fn DoorFlsGetValue(index: u32) -> u64 {
    DoorTlsGetValue(index)
}
#[unsafe(no_mangle)]
pub extern "system" fn DoorFlsSetValue(index: u32, value: u64) -> i32 {
    DoorTlsSetValue(index, value)
}
/// The exiting thread's FLS callbacks (DoorExitThread, process exit).
pub fn run_thread_callbacks() {
    for index in 0..tls::SLOTS as u32 {
        if INDEXES.epoch(index).is_some() {
            run_callback(index);
        }
    }
}
