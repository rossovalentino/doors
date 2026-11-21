//! Private-frame heap admission of a maximize at 3x (3840x2160 output),
//! replayed on a first-fit arena that mirrors the guest heap
//! (`door_rt::heap`: 96 MiB, address-ordered first fit, prefix/suffix
//! splitting, neighbour coalescing, no in-place growth). Only the test
//! thread allocates from the arena; everything else uses the system heap.
#![allow(dead_code, unused_imports)]
extern crate alloc;
// The included Settings sources expect these names at the crate root.
use alloc::{format, string::String};
use door_service_client::Storage;
use door_workspace::menu;
use std::{
    alloc::{GlobalAlloc, Layout, System},
    cell::Cell,
    sync::atomic::{AtomicBool, Ordering},
};
#[path = "../src/icons.rs"]
mod icons;
#[path = "../src/settings.rs"]
mod settings;
#[path = "../src/tokens.rs"]
mod tokens;
#[path = "../src/ui.rs"]
mod ui;
use door_abi::{
    Inherited, Startup,
    platform::{Identity, StartupV2},
};
use door_service_client::Error;
use door_service_protocol::Code;
use door_workspace::{FRAME_MARGIN, admit_frame, grow_frame, settings_layout::Page};

/// The workspace process heap (`desktop/workspace/src/main.rs`).
const HEAP: usize = 96 * 1024 * 1024;
const MAX_EXTENTS: usize = 256;
#[derive(Clone, Copy)]
struct Extent {
    start: usize,
    bytes: usize,
    used: bool,
}
struct Arena {
    base: usize,
    extents: [Extent; MAX_EXTENTS],
    count: usize,
}
static LOCK: AtomicBool = AtomicBool::new(false);
static mut ARENA: Arena = Arena {
    base: 0,
    extents: [Extent {
        start: 0,
        bytes: 0,
        used: false,
    }; MAX_EXTENTS],
    count: 0,
};
thread_local! {
    static TRACKED: Cell<bool> = const { Cell::new(false) };
}
fn with_arena<R>(f: impl FnOnce(&mut Arena) -> R) -> R {
    while LOCK
        .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
        .is_err()
    {
        std::hint::spin_loop();
    }
    // SAFETY: LOCK serializes every access.
    let result = f(unsafe { &mut *core::ptr::addr_of_mut!(ARENA) });
    LOCK.store(false, Ordering::Release);
    result
}
impl Arena {
    fn contains(&self, address: usize) -> bool {
        self.base != 0 && (self.base..self.base + HEAP).contains(&address)
    }
    fn allocate(&mut self, layout: Layout) -> *mut u8 {
        for i in 0..self.count {
            let old = self.extents[i];
            if old.used {
                continue;
            }
            let address = (self.base + old.start + layout.align() - 1) & !(layout.align() - 1);
            let start = address - self.base;
            let end = start + layout.size();
            let old_end = old.start + old.bytes;
            if end > old_end {
                continue;
            }
            let prefix = start > old.start;
            let suffix = end < old_end;
            let added = usize::from(prefix) + usize::from(suffix);
            if self.count + added > MAX_EXTENTS {
                continue;
            }
            self.extents.copy_within(i + 1..self.count, i + 1 + added);
            let mut at = i;
            if prefix {
                self.extents[at] = Extent {
                    start: old.start,
                    bytes: start - old.start,
                    used: false,
                };
                at += 1;
            }
            self.extents[at] = Extent {
                start,
                bytes: layout.size(),
                used: true,
            };
            if suffix {
                self.extents[at + 1] = Extent {
                    start: end,
                    bytes: old_end - end,
                    used: false,
                };
            }
            self.count += added;
            return address as *mut u8;
        }
        core::ptr::null_mut()
    }
    fn deallocate(&mut self, address: usize) {
        let offset = address - self.base;
        let mut i = self.extents[..self.count]
            .iter()
            .position(|e| e.used && e.start == offset)
            .expect("arena allocation");
        self.extents[i].used = false;
        if i > 0 && !self.extents[i - 1].used {
            self.extents[i - 1].bytes += self.extents[i].bytes;
            self.remove(i);
            i -= 1;
        }
        if i + 1 < self.count && !self.extents[i + 1].used {
            self.extents[i].bytes += self.extents[i + 1].bytes;
            self.remove(i + 1);
        }
    }
    fn remove(&mut self, index: usize) {
        self.extents.copy_within(index + 1..self.count, index);
        self.count -= 1;
    }
    fn used(&self) -> usize {
        self.extents[..self.count]
            .iter()
            .filter(|e| e.used)
            .map(|e| e.bytes)
            .sum()
    }
    fn largest_free(&self) -> usize {
        self.extents[..self.count]
            .iter()
            .filter(|e| !e.used)
            .map(|e| e.bytes)
            .max()
            .unwrap_or(0)
    }
}
struct FirstFit;
unsafe impl GlobalAlloc for FirstFit {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if TRACKED.try_with(Cell::get).unwrap_or(false) {
            with_arena(|a| a.allocate(layout))
        } else {
            unsafe { System.alloc(layout) }
        }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        if with_arena(|a| {
            let inside = a.contains(ptr as usize);
            if inside {
                a.deallocate(ptr as usize);
            }
            inside
        }) {
            return;
        }
        unsafe { System.dealloc(ptr, layout) }
    }
}
#[global_allocator]
static GLOBAL: FirstFit = FirstFit;

fn startup() -> StartupV2 {
    StartupV2 {
        base: Startup {
            pid: 1,
            peer_pid: 0,
            role: 1,
            entry: 0,
            tls: 0,
            stack_top: 0,
            inherited: [Inherited::default(); 8],
        },
        identity: Identity {
            uid: 1001,
            ..Identity::default()
        },
        executable_sha256: [0; 32],
        cwd: 0,
        mode: 0,
        arguments: [0; 128],
        argument_bytes: 0,
        environment: [0; 256],
        environment_bytes: 0,
        standard: [door_abi::Inherited::default(); 3],
    }
}
const MIB: f64 = 1024.0 * 1024.0;
fn report(label: &str) {
    let (used, largest) = with_arena(|a| (a.used(), a.largest_free()));
    std::println!(
        "SURFACE_MEMORY {label}: used={:.1}MiB largest_free={:.1}MiB",
        used as f64 / MIB,
        largest as f64 / MIB
    );
}

/// The real Settings session at 3x, then a maximize: admitting the larger
/// frame while the restored-size frame is live (the former order) is refused
/// by first fit; releasing it first (`grow_frame`) succeeds; a refused
/// growth restores a frame of the old length.
#[test]
fn maximizing_settings_at_3x_needs_the_old_frame_released_first() {
    let arena = unsafe { System.alloc(Layout::from_size_align(HEAP, 4096).unwrap()) };
    assert!(!arena.is_null());
    with_arena(|a| {
        a.base = arena as usize;
        a.extents[0] = Extent {
            start: 0,
            bytes: HEAP,
            used: false,
        };
        a.count = 1;
    });
    TRACKED.with(|t| t.set(true));
    let startup = startup();
    let fonts = door_graphics_text::FontAtlas::windows_reference_scaled(3).unwrap();
    let mut settings = settings::Settings::review(&startup, Page::Home, false);
    // Restored Settings at 3x: logical 1129x624, physical 3387x1872.
    settings.configure(1129, 624);
    let images = ui::ImageCache::default();
    let restored = 3387 * 1872 * 4;
    // Window creation: the margin is admitted first, then the frame.
    let mut frame = admit_frame(restored).unwrap();
    // A painted session: icons, text and status allocations made after the
    // margin was released land in its hole (first fit).
    let status = String::from("Window size exceeds available rendering memory.");
    for _ in 0..2 {
        frame.fill(0);
        let mut view = ui::View::new_scaled(&mut frame, 3387, 1872, &fonts, 3).with_images(&images);
        settings.draw(&mut view);
    }
    report("restored session");
    // Maximized: 3840x2016 physical (work area above the 48-DIP taskbar).
    let maximized = 3840 * 2016 * 4;
    let old_order = admit_frame(maximized);
    report("old order attempt");
    assert!(
        matches!(old_order, Err(Error::Service(Code::Resources))),
        "first fit refuses a {:.1} MiB frame plus the {:.1} MiB margin beside the live frame",
        maximized as f64 / MIB,
        FRAME_MARGIN as f64 / MIB
    );
    let mut frame = grow_frame(frame, maximized).expect("released first: admitted");
    assert_eq!(frame.len(), maximized);
    report("maximized");
    {
        frame.fill(0);
        let mut view = ui::View::new_scaled(&mut frame, 3840, 2016, &fonts, 3).with_images(&images);
        settings.configure(1280, 672);
        settings.draw(&mut view);
    }
    // A growth that cannot be admitted keeps a frame of the old length.
    let hog = admit_frame(0).ok().map(|_| {
        let mut v = Vec::<u8>::new();
        let room = with_arena(|a| a.largest_free());
        v.try_reserve_exact(room.saturating_sub(4096)).unwrap();
        v
    });
    match grow_frame(frame, maximized + 4096) {
        Err((old, Error::Service(Code::Resources))) => assert_eq!(old.len(), maximized),
        other => panic!("expected refusal, got {:?}", other.map(|f| f.len())),
    }
    drop((hog, status, images, settings, fonts));
    TRACKED.with(|t| t.set(false));
}
