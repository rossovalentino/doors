//! Firmware handles exist only inside these functions and drop before final exit.
use door_boot_protocol::Framebuffer;
use uefi::{
    CStr16,
    boot::{self, AllocateType, MemoryType},
    proto::{
        console::gop::{GraphicsOutput, PixelFormat},
        media::file::{File, FileAttribute, FileInfo, FileMode},
    },
};

pub struct Allocation {
    base: u64,
    bytes: usize,
}
impl Allocation {
    pub fn new(bytes: usize) -> Self {
        Self::below(bytes, 0xffff_ffff)
    }
    /// Allocate actual free firmware pages for a SIPI page and adjacent mailbox.
    /// MaxAddress bounds the allocation's last byte, not just its starting address.
    pub fn secondary_startup() -> Self {
        Self::below(8192, 0x000f_ffff)
    }
    fn below(bytes: usize, maximum: u64) -> Self {
        assert!(bytes > 0 && bytes <= 256 * 1024 * 1024);
        let bytes = bytes.div_ceil(4096) * 4096;
        let pointer = boot::allocate_pages(
            AllocateType::MaxAddress(maximum),
            MemoryType::LOADER_DATA,
            bytes / 4096,
        )
        .expect("allocate pages");
        let base = pointer.as_ptr() as u64;
        assert!(
            base.is_multiple_of(4096)
                && base != 0
                && base
                    .checked_add(bytes as u64 - 1)
                    .is_some_and(|end| end <= maximum),
            "firmware allocation extent"
        );
        // SAFETY: fresh exclusive firmware-owned pages, initialized before borrowing.
        unsafe {
            core::ptr::write_bytes(pointer.as_ptr(), 0, bytes);
        }
        Self { base, bytes }
    }
    pub fn data(&self) -> &[u8] {
        unsafe { core::slice::from_raw_parts(self.base as *const u8, self.bytes) }
    }
    pub fn base(&self) -> u64 {
        self.base
    }
    pub fn bytes(&self) -> usize {
        self.bytes
    }
    pub fn data_mut(&mut self) -> &mut [u8] {
        unsafe { core::slice::from_raw_parts_mut(self.base as *mut u8, self.bytes) }
    }
}
pub fn read(path: &CStr16, limit: usize) -> (Allocation, usize) {
    let mut filesystem =
        boot::get_image_file_system(boot::image_handle()).expect("boot filesystem");
    let mut root = filesystem.open_volume().expect("root volume");
    let mut file = root
        .open(path, FileMode::Read, FileAttribute::empty())
        .expect("boot file")
        .into_regular_file()
        .expect("regular file");
    #[repr(align(8))]
    struct Info([u8; 1024]);
    let mut info = Info([0; 1024]);
    let size = usize::try_from(
        file.get_info::<FileInfo>(&mut info.0)
            .expect("file information")
            .file_size(),
    )
    .expect("file size");
    assert!(size > 0 && size <= limit, "boot file size");
    let mut allocation = Allocation::new(size);
    let read = file
        .read(&mut allocation.data_mut()[..size])
        .expect("boot file read");
    assert_eq!(read, size, "short boot read");
    let mut extra = [0; 1];
    assert_eq!(
        file.read(&mut extra).expect("file eof"),
        0,
        "file changed during read"
    );
    (allocation, size)
}
pub fn framebuffer(request: &[u8]) -> (Framebuffer, bool) {
    let desired: (usize, usize) = match request {
        b"1280x720" => (1280, 720),
        b"1920x1080" => (1920, 1080),
        b"1920x1200" => (1920, 1200),
        b"2560x1440" => (2560, 1440),
        b"2560x1600" => (2560, 1600),
        b"3840x2160" => (3840, 2160),
        _ => (1024, 768),
    };
    if let Some(framebuffer) = crate::ramfb::framebuffer(desired.0 as u32, desired.1 as u32) {
        return (framebuffer, true);
    }
    let handle = boot::get_handle_for_protocol::<GraphicsOutput>().expect("GOP handle");
    let mut gop = boot::open_protocol_exclusive::<GraphicsOutput>(handle).expect("GOP protocol");
    let mode = gop
        .modes()
        .find(|mode| {
            mode.info().resolution() == desired
                && matches!(
                    mode.info().pixel_format(),
                    PixelFormat::Rgb | PixelFormat::Bgr
                )
        })
        .or_else(|| {
            gop.modes().find(|mode| {
                mode.info().resolution() == (1024, 768)
                    && matches!(
                        mode.info().pixel_format(),
                        PixelFormat::Rgb | PixelFormat::Bgr
                    )
            })
        })
        .expect("supported GOP desktop mode");
    gop.set_mode(&mode).expect("GOP mode");
    let info = gop.current_mode_info();
    let format = match info.pixel_format() {
        PixelFormat::Rgb => 0,
        PixelFormat::Bgr => 1,
        _ => panic!("unsupported GOP pixel format"),
    };
    let (width, height) = info.resolution();
    let mut backing = gop.frame_buffer();
    let physical = backing.as_mut_ptr() as u64;
    let result = Framebuffer {
        physical,
        bytes: backing.size() as u64,
        width: width as u32,
        height: height as u32,
        stride: info.stride() as u32,
        format,
        virtual_start: physical,
    };
    let mut encoded = [0; 40];
    result.encode(&mut encoded).expect("framebuffer extent");
    (result, false)
}
