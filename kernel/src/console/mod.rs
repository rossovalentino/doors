mod font;
use core::sync::atomic::{AtomicU64, Ordering};
use door_boot_protocol::Framebuffer;
static PANIC_BASE: AtomicU64 = AtomicU64::new(0);
static PANIC_BYTES: AtomicU64 = AtomicU64::new(0);
static PANIC_DIMENSIONS: AtomicU64 = AtomicU64::new(0);
static PANIC_FORMAT: AtomicU64 = AtomicU64::new(0);

pub fn emergency() {
    let base = PANIC_BASE.load(Ordering::Acquire);
    if base == 0 {
        return;
    }
    let dimensions = PANIC_DIMENSIONS.load(Ordering::Relaxed);
    let encoding = PANIC_FORMAT.load(Ordering::Relaxed);
    let fb = Framebuffer {
        physical: base,
        virtual_start: base,
        bytes: PANIC_BYTES.load(Ordering::Relaxed),
        width: dimensions as u32,
        height: (dimensions >> 32) as u32,
        stride: encoding as u32,
        format: (encoding >> 32) as u32,
    };
    // SAFETY: the boot CPU publishes only a validated reserved mapping; IRQs and
    // SMP remain disabled. Panic does not borrow a potentially active console.
    let mut console = Console { fb };
    console.rect(0, 0, fb.width, fb.height, 0x381724);
    console.text(42, 60, "DOOR OS / KERNEL PANIC", 3, 0xff8299);
    console.text(
        42,
        130,
        "CONTROLLED FAILURE. SERIAL EVIDENCE CAPTURED.",
        2,
        0xf1f5fb,
    );
    console.text(
        42,
        190,
        "RESTART THE DIAGNOSTIC VM TO CONTINUE.",
        2,
        0xf1f5fb,
    );
}

pub struct Console {
    fb: Framebuffer,
}
impl Console {
    pub fn framebuffer(&self) -> Framebuffer {
        self.fb
    }
    /// Convert a checked contiguous logical-pixel span into the actual GOP
    /// layout. This is a CPU transfer, without page flip/vblank guarantees.
    pub fn present_pixels(&mut self, first: u64, pixels: &[u8]) -> Result<(), door_abi::Status> {
        let total = u64::from(self.fb.width) * u64::from(self.fb.height);
        if !pixels.len().is_multiple_of(4)
            || first
                .checked_add((pixels.len() / 4) as u64)
                .is_none_or(|end| end > total)
        {
            return Err(door_abi::Status::InvalidArgument);
        }
        // Divide once per span. Per-pixel 64-bit division made CPU GOP copies
        // dominate input latency under TCG, despite already validated extents.
        let mut x = first % u64::from(self.fb.width);
        let mut y = first / u64::from(self.fb.width);
        if self.fb.format == 1 {
            // GOP BGRX accepts the producer's BGRA bytes directly; its fourth
            // byte is reserved. Copy scanline spans with REP instead of one
            // volatile store, bounds calculation and branch per pixel. Inline
            // assembly is volatile and declares its memory effects. The trusted
            // mapping is exclusively owned and the complete logical span was
            // checked above; padding between physical rows is never touched.
            let mut source = pixels.as_ptr();
            let mut remaining = pixels.len() / 4;
            while remaining != 0 {
                let count = remaining.min((u64::from(self.fb.width) - x) as usize);
                let offset = (y * u64::from(self.fb.stride) + x) * 4;
                unsafe {
                    core::arch::asm!(
                        "rep movsd",
                        inout("rsi") source => _,
                        inout("rdi") (self.fb.virtual_start + offset) as *mut u8 => _,
                        inout("rcx") count => _,
                        options(nostack, preserves_flags),
                    );
                    source = source.add(count * 4);
                }
                remaining -= count;
                x = 0;
                y += 1;
            }
            return Ok(());
        }
        for bytes in pixels.chunks_exact(4) {
            let mut color = u32::from_le_bytes(bytes.try_into().unwrap()) & 0xffffff;
            if self.fb.format == 0 {
                color = ((color & 255) << 16) | (color & 0xff00) | ((color >> 16) & 255)
            }
            let offset = (y * u64::from(self.fb.stride) + x) * 4;
            // Checked boot mapping and logical span keep each write in the
            // reserved output, including padded physical strides.
            unsafe {
                core::ptr::write_volatile((self.fb.virtual_start + offset) as *mut u32, color)
            }
            x += 1;
            if x == u64::from(self.fb.width) {
                x = 0;
                y += 1;
            }
        }
        Ok(())
    }
    /// Render the owned text-terminal state. The session console broker is the
    /// sole writer during native service execution; the boot console resumes
    /// ownership only after all service contexts retire.
    pub fn terminal_theme(&mut self, rows: &[[u8; 80]; 28], column: usize, row: usize, theme: u64) {
        let (background, foreground, accent) = match theme {
            1 => (0xf3f5f7, 0x18202a, 0x006b61),
            2 => (0x080808, 0xffffff, 0xffd400),
            _ => (0x101827, 0xe0eaf4, 0x56d4be),
        };
        self.rect(0, 0, self.fb.width, self.fb.height, background);
        self.rect(0, 0, self.fb.width, 5, accent);
        self.text(24, 22, "DOOR OS / NATIVE SERVICE CONSOLE", 2, accent);
        for (index, line) in rows.iter().enumerate() {
            let text = core::str::from_utf8(line).expect("terminal ASCII cells");
            self.text(24, 62 + index as u32 * 21, text, 2, foreground);
        }
        self.rect(
            24 + column.min(79) as u32 * 12,
            76 + row.min(27) as u32 * 21,
            10,
            2,
            0x56d4be,
        );
        self.text(
            24,
            690,
            "PROTECTED NATIVE USER SERVICES / PRIVATE DEVELOPMENT DISK",
            1,
            0x97adc6,
        );
    }
    /// # Safety
    /// The checked framebuffer is mapped with writable device attributes, owned
    /// exclusively by this boot CPU, and remains reserved for the object's lifetime.
    pub unsafe fn new(fb: Framebuffer) -> Self {
        PANIC_BYTES.store(fb.bytes, Ordering::Relaxed);
        PANIC_DIMENSIONS.store(
            u64::from(fb.width) | (u64::from(fb.height) << 32),
            Ordering::Relaxed,
        );
        PANIC_FORMAT.store(
            u64::from(fb.stride) | (u64::from(fb.format) << 32),
            Ordering::Relaxed,
        );
        PANIC_BASE.store(fb.physical, Ordering::Release);
        Self { fb }
    }
    fn rect(&mut self, x: u32, y: u32, w: u32, h: u32, color: u32) {
        let color = if self.fb.format == 0 {
            ((color & 255) << 16) | (color & 0xff00) | ((color >> 16) & 255)
        } else {
            color
        };
        for py in y..y.saturating_add(h).min(self.fb.height) {
            for px in x..x.saturating_add(w).min(self.fb.width) {
                let offset = (u64::from(py) * u64::from(self.fb.stride) + u64::from(px)) * 4;
                // Dimensions/stride/extent were validated before construction; clipping
                // above keeps every volatile device write within the mapped byte extent.
                unsafe {
                    core::ptr::write_volatile((self.fb.virtual_start + offset) as *mut u32, color);
                }
            }
        }
    }
    /// Desktop boot splash: the Doors mark centered on black (as Windows shows
    /// its logo), sized to the output. Diagnostic modes keep `screen`.
    pub fn splash(&mut self) {
        static LOGO_120: &[u8] = include_bytes!("../../../assets/branding/door.120.bgra");
        static LOGO_160: &[u8] = include_bytes!("../../../assets/branding/door.160.bgra");
        static LOGO_288: &[u8] = include_bytes!("../../../assets/branding/door.288.bgra");
        self.rect(0, 0, self.fb.width, self.fb.height, 0);
        let (side, logo) = match self.fb.height {
            h if h >= 1800 => (288, LOGO_288),
            h if h >= 900 => (160, LOGO_160),
            _ => (120, LOGO_120),
        };
        if self.fb.width < side || self.fb.height < side {
            return;
        }
        // Slightly above center, where Windows places its boot logo.
        let left = (self.fb.width - side) / 2;
        let top = (self.fb.height * 2 / 5).saturating_sub(side / 2);
        for (i, p) in logo.chunks_exact(4).enumerate() {
            if p[3] == 0 {
                continue;
            }
            // Premultiplied BGRA over black is the color itself.
            let color = (u32::from(p[2]) << 16) | (u32::from(p[1]) << 8) | u32::from(p[0]);
            let (x, y) = (left + (i as u32 % side), top + (i as u32 / side));
            self.rect(x, y, 1, 1, color);
        }
        // Windows 11 26H2's unified "donut" boot indicator sits low on the
        // screen. Init has no redraw loop, so this is one still frame.
        let diameter = side * 3 / 10;
        let cy = self.fb.height * 17 / 25;
        if cy + diameter / 2 + 1 < self.fb.height {
            self.donut(self.fb.width / 2, cy, diameter);
        }
    }
    /// A dim ring track with a bright quarter arc, anti-aliased by distance
    /// from the stroke centerline in 1/16 px over the black splash.
    fn donut(&mut self, cx: u32, cy: u32, diameter: u32) {
        fn isqrt(n: u64) -> u64 {
            if n < 2 {
                return n;
            }
            let (mut x, mut y) = (n, n.div_ceil(2));
            while y < x {
                x = y;
                y = (x + n / x) / 2;
            }
            x
        }
        let stroke = i64::from((diameter / 9).max(3)) * 16;
        let mid = (i64::from(diameter) * 16 - stroke) / 2;
        let reach = i64::from(diameter / 2 + 1);
        for y in -reach..=reach {
            for x in -reach..=reach {
                let (fx, fy) = (x * 16 + 8, y * 16 + 8);
                let d = isqrt((fx * fx + fy * fy) as u64) as i64;
                let coverage = (stroke / 2 + 8 - (d - mid).abs()).clamp(0, 16);
                if coverage == 0 {
                    continue;
                }
                let level = if fx >= 0 && fy <= 0 { 255 } else { 58 };
                let v = (level * coverage / 16) as u32;
                let (px, py) = (i64::from(cx) + x, i64::from(cy) + y);
                if px >= 0 && py >= 0 {
                    self.rect(px as u32, py as u32, 1, 1, v * 0x01_01_01);
                }
            }
        }
    }
    fn text(&mut self, x: u32, y: u32, text: &str, size: u32, color: u32) {
        for (index, byte) in text.bytes().enumerate() {
            for (row, bits) in font::glyph(byte).into_iter().enumerate() {
                for column in 0..5 {
                    if bits & (1 << (4 - column)) != 0 {
                        self.rect(
                            x + (index as u32 * 6 + column) * size,
                            y + row as u32 * size,
                            size,
                            size,
                            color,
                        );
                    }
                }
            }
        }
    }
    pub fn screen(&mut self, detail: bool) {
        self.rect(0, 0, self.fb.width, self.fb.height, 0x101827);
        self.rect(0, 0, self.fb.width, 8, 0x56d4be);
        self.rect(42, 46, 42, 60, 0x56d4be);
        self.rect(49, 53, 27, 46, 0x101827);
        self.rect(68, 78, 4, 4, 0x56d4be);
        self.text(110, 52, "DOOR OS", 4, 0xf1f5fb);
        self.text(110, 94, "P05 / PROTECTED NATIVE EXECUTION", 2, 0x97adc6);
        self.rect(42, 154, self.fb.width.saturating_sub(84), 244, 0x19273a);
        self.text(66, 180, "KERNEL IS RUNNING", 3, 0x56d4be);
        for (row, text) in [
            "SIX PROTECTED USER PROCESSES",
            "AUTHENTICATED IPC AND SHARED MEMORY",
            "CONTAINED FAULTS / SURVIVOR VERIFIED",
            "NATIVE ABI AND CLEANUP VERIFIED",
        ]
        .iter()
        .enumerate()
        {
            self.text(66, 235 + row as u32 * 36, text, 2, 0xe0eaf4);
        }
        self.text(
            42,
            438,
            if detail {
                "DIAGNOSTICS: USER ISOLATION CHECKS PASSED"
            } else {
                "EARLY DIAGNOSTIC CONSOLE"
            },
            2,
            0xf1f5fb,
        );
        self.text(
            42,
            480,
            if detail {
                "PROCESS FRAMES RETURNED AFTER RETIREMENT."
            } else {
                "DESKTOP AND INTERACTIVE APPS ARRIVE LATER."
            },
            2,
            0x97adc6,
        );
        self.text(
            42,
            548,
            "H HELP   D DIAGNOSTICS   P PANIC   R REBOOT",
            2,
            0x56d4be,
        );
        self.text(
            42,
            594,
            "LOCAL DEVELOPMENT / NO NETWORK / NO USER STORAGE",
            1,
            0x97adc6,
        );
    }
}
