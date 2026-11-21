//! The console screen buffer as Windows programs see it (System.Console,
//! PowerShell's host and PSReadLine, C runtimes), over Doors' console:
//! the Terminal tab's grid, which reads virtual terminal sequences as
//! Windows Terminal does, behind the console service that follows the
//! same output and keeps the cursor and the input mode (ntdll's
//! `DoorConsole*` calls).
//!
//! The buffer is the Terminal's grid, as Windows Terminal's pseudoconsole
//! buffer is its window: GetConsoleScreenBufferInfo reports the grid's
//! size and the cursor where the output left it. The legacy calls that
//! position, colour or fill the screen send the VT sequences that do the
//! same (SetConsoleCursorPosition is CUP, SetConsoleTextAttribute is SGR,
//! FillConsoleOutputCharacter erases or writes the cells), as conhost's
//! VT pass-through does. Cells are not read back (ReadConsoleOutput gives
//! blanks). Input is ntdll's: key events (ReadConsoleInputW), and lines
//! edited and echoed as conhost edits them (ReadConsoleW, with what does
//! not fit kept for the next call).
use core::sync::atomic::{AtomicBool, Ordering};
use door_windows_abi as abi;

#[link(name = "ntdll", kind = "dylib")]
unsafe extern "system" {
    fn RtlSetLastWin32Error(value: u32);
    fn RtlGetLastWin32Error() -> u32;
    fn DoorConsoleState(out: u64) -> i32;
    fn DoorConsoleReadKeys(records: u64, length: u32, count: u64, flags: u32) -> i32;
    fn DoorConsoleInputEvents(count: u64) -> i32;
    fn DoorConsoleFlushInput() -> i32;
    fn DoorConsoleWriteKeys(records: u64, count: u32) -> i32;
}

const ERROR_INVALID_HANDLE: u32 = 6;
const ERROR_INVALID_PARAMETER: u32 = 87;

struct State {
    attributes: u16,
    cursor_size: u32,
    title: [u16; 256],
    title_len: usize,
    /// Units of the last line read, not yet returned by ReadConsoleW.
    keys: [u16; 512],
    key_start: usize,
    key_len: usize,
}
struct Shared {
    held: AtomicBool,
    state: core::cell::UnsafeCell<State>,
}
unsafe impl Sync for Shared {}
static STATE: Shared = Shared {
    held: AtomicBool::new(false),
    state: core::cell::UnsafeCell::new(State {
        attributes: 0x07,
        cursor_size: 25,
        title: [0; 256],
        title_len: 0,
        keys: [0; 512],
        key_start: 0,
        key_len: 0,
    }),
};
fn with<R>(f: impl FnOnce(&mut State) -> R) -> R {
    while STATE.held.swap(true, Ordering::Acquire) {
        core::hint::spin_loop();
    }
    // SAFETY: `held` gives this closure the only access.
    let r = f(unsafe { &mut *STATE.state.get() });
    STATE.held.store(false, Ordering::Release);
    r
}
fn fail(code: u32) -> i32 {
    unsafe { RtlSetLastWin32Error(code) };
    0
}
fn output(handle: usize) -> bool {
    matches!(abi::widen(handle), abi::OUTPUT_HANDLE | abi::ERROR_HANDLE)
}
fn input(handle: usize) -> bool {
    abi::widen(handle) == abi::INPUT_HANDLE
}
fn write<T: Copy>(pointer: usize, value: T) {
    if pointer != 0 {
        // SAFETY: the caller's out parameter of type T.
        unsafe { (pointer as *mut T).write_unaligned(value) };
    }
}
/// COORD passed by value: X in the low half, Y in the high half.
fn coord(value: u32) -> (i16, i16) {
    (value as u16 as i16, (value >> 16) as u16 as i16)
}
fn pack(x: i16, y: i16) -> u32 {
    u32::from(x as u16) | (u32::from(y as u16) << 16)
}

/// The screen: its size and the cursor (the console service's state).
#[derive(Clone, Copy)]
struct Screen {
    columns: i16,
    rows: i16,
    x: i16,
    y: i16,
    visible: bool,
}
fn screen() -> Screen {
    let mut bytes = [0u8; 20];
    if unsafe { DoorConsoleState(bytes.as_mut_ptr() as u64) } == 0 {
        // No console service answer: Windows Terminal's default window.
        return Screen {
            columns: 120,
            rows: 30,
            x: 0,
            y: 0,
            visible: true,
        };
    }
    let half = |i: usize| u16::from_le_bytes([bytes[2 * i], bytes[2 * i + 1]]) as i16;
    let flags = u32::from_le_bytes(bytes[12..16].try_into().unwrap());
    Screen {
        columns: half(0),
        rows: half(1),
        x: half(2),
        y: half(3),
        visible: flags & 1 != 0,
    }
}

/// Text (virtual terminal sequences) to the console, as the program's own
/// output.
fn send(text: &[u8]) {
    let mut written = 0u32;
    crate::guest::WriteFile(
        abi::narrow(abi::OUTPUT_HANDLE),
        text.as_ptr() as usize,
        text.len() as u32,
        &mut written as *mut u32 as usize,
        0,
    );
}
/// A small buffer for building sequences.
struct Out {
    bytes: [u8; 512],
    len: usize,
}
impl Out {
    const fn new() -> Self {
        Self {
            bytes: [0; 512],
            len: 0,
        }
    }
    fn push(&mut self, text: &[u8]) {
        if self.len + text.len() > self.bytes.len() {
            self.flush();
        }
        let n = text.len().min(self.bytes.len());
        self.bytes[self.len..self.len + n].copy_from_slice(&text[..n]);
        self.len += n;
    }
    fn number(&mut self, n: u32) {
        let mut digits = [0u8; 10];
        let mut at = digits.len();
        let mut n = n;
        loop {
            at -= 1;
            digits[at] = b'0' + (n % 10) as u8;
            n /= 10;
            if n == 0 {
                break;
            }
        }
        let text = digits;
        self.push(&text[at..]);
    }
    fn char(&mut self, unit: u16) {
        let c = char::from_u32(u32::from(unit)).unwrap_or('\u{fffd}');
        let mut bytes = [0u8; 4];
        let text = c.encode_utf8(&mut bytes).as_bytes();
        let mut copy = [0u8; 4];
        copy[..text.len()].copy_from_slice(text);
        self.push(&copy[..text.len()]);
    }
    /// CUP to a zero-based cell.
    fn cup(&mut self, x: i16, y: i16) {
        self.push(b"\x1b[");
        self.number(y as u32 + 1);
        self.push(b";");
        self.number(x as u32 + 1);
        self.push(b"H");
    }
    /// SGR for a console attribute word.
    fn sgr(&mut self, attributes: u16) {
        // Console colour bits are blue, green, red, intensity; VT's are
        // red, green, blue.
        let ansi = |w: u16| (w & 2) | ((w & 1) << 2) | ((w & 4) >> 2);
        self.push(b"\x1b[0");
        if attributes & 0xff != 0x07 {
            let fg = attributes & 0xf;
            let bg = (attributes >> 4) & 0xf;
            self.push(b";");
            self.number(u32::from(ansi(fg & 7)) + if fg & 8 != 0 { 90 } else { 30 });
            self.push(b";");
            self.number(u32::from(ansi(bg & 7)) + if bg & 8 != 0 { 100 } else { 40 });
        }
        if attributes & 0x4000 != 0 {
            self.push(b";7");
        }
        if attributes & 0x8000 != 0 {
            self.push(b";4");
        }
        self.push(b"m");
    }
    fn flush(&mut self) {
        if self.len > 0 {
            let bytes = self.bytes;
            send(&bytes[..self.len]);
            self.len = 0;
        }
    }
}

/// CONSOLE_SCREEN_BUFFER_INFO: size, cursor, attributes, window, maximum.
fn buffer_info(s: &Screen, attributes: u16) -> [u8; 22] {
    let mut out = [0u8; 22];
    let words = [
        s.columns,
        s.rows,
        s.x,
        s.y,
        attributes as i16,
        0,
        0,
        s.columns - 1,
        s.rows - 1,
        s.columns,
        s.rows,
    ];
    for (i, w) in words.iter().enumerate() {
        out[2 * i..2 * i + 2].copy_from_slice(&w.to_le_bytes());
    }
    out
}
#[unsafe(no_mangle)]
pub extern "system" fn GetConsoleScreenBufferInfo(handle: usize, info: usize) -> i32 {
    if !output(handle) {
        return fail(ERROR_INVALID_HANDLE);
    }
    if info == 0 {
        return fail(ERROR_INVALID_PARAMETER);
    }
    let attributes = with(|s| s.attributes);
    write(info, buffer_info(&screen(), attributes));
    1
}
/// Windows Terminal's Campbell colour table (COLORREF, 0x00BBGGRR).
const CAMPBELL: [u32; 16] = [
    0x000C_0C0C,
    0x00DA_3700,
    0x000E_A113,
    0x00DD_963A,
    0x001F_0FC5,
    0x0098_1788,
    0x0000_9CC1,
    0x00CC_CCCC,
    0x0076_7676,
    0x00FF_783B,
    0x000C_C616,
    0x00D6_D661,
    0x0056_48E7,
    0x009E_00B4,
    0x00A5_F1F9,
    0x00F2_F2F2,
];
/// CONSOLE_SCREEN_BUFFER_INFOEX (96 bytes): cbSize, the basic information,
/// popup attributes, full-screen support and the colour table.
#[unsafe(no_mangle)]
pub extern "system" fn GetConsoleScreenBufferInfoEx(handle: usize, info: usize) -> i32 {
    if !output(handle) {
        return fail(ERROR_INVALID_HANDLE);
    }
    if info == 0 || unsafe { (info as *const u32).read_unaligned() } != 96 {
        return fail(ERROR_INVALID_PARAMETER);
    }
    let attributes = with(|s| s.attributes);
    let mut out = [0u8; 96];
    out[..4].copy_from_slice(&96u32.to_le_bytes());
    out[4..26].copy_from_slice(&buffer_info(&screen(), attributes));
    out[26..28].copy_from_slice(&0xf5u16.to_le_bytes());
    for (i, color) in CAMPBELL.iter().enumerate() {
        out[32 + 4 * i..36 + 4 * i].copy_from_slice(&color.to_le_bytes());
    }
    write(info, out);
    1
}
#[unsafe(no_mangle)]
pub extern "system" fn SetConsoleScreenBufferInfoEx(handle: usize, info: usize) -> i32 {
    if !output(handle) {
        return fail(ERROR_INVALID_HANDLE);
    }
    if info == 0 {
        return fail(ERROR_INVALID_PARAMETER);
    }
    let attributes = unsafe { ((info + 12) as *const u16).read_unaligned() };
    SetConsoleTextAttribute(handle, attributes)
}
/// CUP, as conhost's VT pass-through moves the cursor.
#[unsafe(no_mangle)]
pub extern "system" fn SetConsoleCursorPosition(handle: usize, position: u32) -> i32 {
    if !output(handle) {
        return fail(ERROR_INVALID_HANDLE);
    }
    let (x, y) = coord(position);
    let s = screen();
    if x < 0 || y < 0 || x >= s.columns || y >= s.rows {
        return fail(ERROR_INVALID_PARAMETER);
    }
    let mut out = Out::new();
    out.cup(x, y);
    out.flush();
    1
}
#[unsafe(no_mangle)]
pub extern "system" fn GetConsoleCursorInfo(handle: usize, info: usize) -> i32 {
    if !output(handle) {
        return fail(ERROR_INVALID_HANDLE);
    }
    let size = with(|s| s.cursor_size);
    write(info, size);
    write(info + 4, i32::from(screen().visible));
    1
}
#[unsafe(no_mangle)]
pub extern "system" fn SetConsoleCursorInfo(handle: usize, info: usize) -> i32 {
    if !output(handle) {
        return fail(ERROR_INVALID_HANDLE);
    }
    if info == 0 {
        return fail(ERROR_INVALID_PARAMETER);
    }
    let size = unsafe { (info as *const u32).read_unaligned() };
    let visible = unsafe { ((info + 4) as *const i32).read_unaligned() } != 0;
    if !(1..=100).contains(&size) {
        return fail(ERROR_INVALID_PARAMETER);
    }
    with(|s| s.cursor_size = size);
    send(if visible { b"\x1b[?25h" } else { b"\x1b[?25l" });
    1
}
/// SGR in the attribute's colours (the console colour table is Windows
/// Terminal's, so index 7 on 0 is the scheme's default pen).
#[unsafe(no_mangle)]
pub extern "system" fn SetConsoleTextAttribute(handle: usize, attributes: u16) -> i32 {
    if !output(handle) {
        return fail(ERROR_INVALID_HANDLE);
    }
    with(|s| s.attributes = attributes);
    let mut out = Out::new();
    out.sgr(attributes);
    out.flush();
    1
}
/// The cells from `position` on, at most `length` (to the screen's end),
/// filled with `unit` (a blank erases): the cursor stays where it was.
fn fill(handle: usize, unit: Option<u16>, length: u32, position: u32, written: usize) -> i32 {
    if !output(handle) {
        return fail(ERROR_INVALID_HANDLE);
    }
    let (x, y) = coord(position);
    let s = screen();
    if x < 0 || y < 0 || x >= s.columns || y >= s.rows {
        return fail(ERROR_INVALID_PARAMETER);
    }
    let left = (i32::from(s.rows - y) * i32::from(s.columns) - i32::from(x)) as u32;
    let cells = length.min(left);
    if let Some(unit) = unit {
        let mut out = Out::new();
        out.push(b"\x1b7");
        let whole = x == 0 && y == 0 && cells == left;
        if whole && unit == u16::from(b' ') {
            out.push(b"\x1b[2J");
        } else {
            let (mut cx, mut cy, mut remaining) = (x, y, cells);
            while remaining > 0 {
                let run = remaining.min((s.columns - cx) as u32);
                out.cup(cx, cy);
                if unit == u16::from(b' ') {
                    out.push(b"\x1b[");
                    out.number(run);
                    out.push(b"X");
                } else {
                    for _ in 0..run {
                        out.char(unit);
                    }
                }
                remaining -= run;
                cx = 0;
                cy += 1;
            }
        }
        out.push(b"\x1b8");
        out.flush();
    }
    write(written, cells);
    1
}
#[unsafe(no_mangle)]
pub extern "system" fn FillConsoleOutputCharacterW(
    handle: usize,
    character: u16,
    length: u32,
    position: u32,
    written: usize,
) -> i32 {
    fill(handle, Some(character), length, position, written)
}
#[unsafe(no_mangle)]
pub extern "system" fn FillConsoleOutputCharacterA(
    handle: usize,
    character: u8,
    length: u32,
    position: u32,
    written: usize,
) -> i32 {
    fill(
        handle,
        Some(u16::from(character)),
        length,
        position,
        written,
    )
}
/// Colours of cells already written cannot change through VT; the count
/// is reported (programs pair this with a fill of blanks, which take the
/// current colours).
#[unsafe(no_mangle)]
pub extern "system" fn FillConsoleOutputAttribute(
    handle: usize,
    _attributes: u16,
    length: u32,
    position: u32,
    written: usize,
) -> i32 {
    fill(handle, None, length, position, written)
}
/// WriteConsoleOutputCharacterW: characters from `position` on, the cursor
/// staying where it was.
#[unsafe(no_mangle)]
pub extern "system" fn WriteConsoleOutputCharacterW(
    handle: usize,
    characters: usize,
    length: u32,
    position: u32,
    written: usize,
) -> i32 {
    if !output(handle) {
        return fail(ERROR_INVALID_HANDLE);
    }
    if characters == 0 && length != 0 {
        return fail(ERROR_INVALID_PARAMETER);
    }
    let (x, y) = coord(position);
    let s = screen();
    if x < 0 || y < 0 || x >= s.columns || y >= s.rows {
        return fail(ERROR_INVALID_PARAMETER);
    }
    let left = (i32::from(s.rows - y) * i32::from(s.columns) - i32::from(x)) as u32;
    let count = length.min(left);
    let mut out = Out::new();
    out.push(b"\x1b7");
    let (mut cx, mut cy) = (x, y);
    out.cup(cx, cy);
    for i in 0..count as usize {
        let unit = unsafe { (characters as *const u16).add(i).read_unaligned() };
        out.char(unit);
        cx += 1;
        if cx == s.columns {
            cx = 0;
            cy += 1;
            if cy < s.rows {
                out.cup(cx, cy);
            }
        }
    }
    out.push(b"\x1b8");
    out.flush();
    write(written, count);
    1
}
/// WriteConsoleOutputW: a rectangle of CHAR_INFO cells drawn at `region`
/// (clipped to the screen; the clipped region goes back), each cell in
/// its attribute's colours; the cursor and pen stay as they were.
#[unsafe(no_mangle)]
pub extern "system" fn WriteConsoleOutputW(
    handle: usize,
    buffer: usize,
    size: u32,
    origin: u32,
    region: usize,
) -> i32 {
    if !output(handle) {
        return fail(ERROR_INVALID_HANDLE);
    }
    if buffer == 0 || region == 0 {
        return fail(ERROR_INVALID_PARAMETER);
    }
    let (columns, rows) = coord(size);
    let (from_x, from_y) = coord(origin);
    let r = |i: usize| unsafe { ((region + 2 * i) as *const i16).read_unaligned() };
    let s = screen();
    let (left, top) = (r(0).max(0), r(1).max(0));
    let right = r(2).min(s.columns - 1).min(left + columns - from_x - 1);
    let bottom = r(3).min(s.rows - 1).min(top + rows - from_y - 1);
    for (i, value) in [left, top, right, bottom].into_iter().enumerate() {
        write(region + 2 * i, value);
    }
    if right < left || bottom < top {
        return 1;
    }
    let mut out = Out::new();
    out.push(b"\x1b7");
    let mut pen = None;
    for y in top..=bottom {
        out.cup(left, y);
        for x in left..=right {
            let index =
                (y - top + from_y) as usize * columns as usize + (x - left + from_x) as usize;
            let cell = unsafe { ((buffer + 4 * index) as *const u32).read_unaligned() };
            let (unit, attributes) = (cell as u16, (cell >> 16) as u16);
            if pen != Some(attributes) {
                out.sgr(attributes);
                pen = Some(attributes);
            }
            out.char(if unit == 0 { u16::from(b' ') } else { unit });
        }
    }
    out.sgr(with(|s| s.attributes));
    out.push(b"\x1b8");
    out.flush();
    1
}
/// ReadConsoleOutputW: the cells are the Terminal's; blanks in the current
/// attributes come back, for the region clipped to the screen.
#[unsafe(no_mangle)]
pub extern "system" fn ReadConsoleOutputW(
    handle: usize,
    buffer: usize,
    size: u32,
    _origin: u32,
    region: usize,
) -> i32 {
    if !output(handle) {
        return fail(ERROR_INVALID_HANDLE);
    }
    if buffer == 0 || region == 0 {
        return fail(ERROR_INVALID_PARAMETER);
    }
    let (columns, rows) = coord(size);
    let r = |i: usize| unsafe { ((region + 2 * i) as *const i16).read_unaligned() };
    let s = screen();
    let attributes = with(|s| s.attributes);
    let (left, top) = (r(0), r(1));
    let right = r(2).min(s.columns - 1);
    let bottom = r(3).min(s.rows - 1);
    for (i, value) in [left, top, right, bottom].into_iter().enumerate() {
        write(region + 2 * i, value);
    }
    for index in 0..(i32::from(columns.max(0)) * i32::from(rows.max(0))) as usize {
        write(
            buffer + 4 * index,
            u32::from(b' ') | (u32::from(attributes) << 16),
        );
    }
    1
}
/// ScrollConsoleScreenBufferW for whole rows (how programs scroll a
/// region): the rows move as a scrolling region scrolls, blank rows in
/// the current colours entering. Partial-width rectangles are not moved.
#[unsafe(no_mangle)]
pub extern "system" fn ScrollConsoleScreenBufferW(
    handle: usize,
    scroll: usize,
    _clip: usize,
    destination: u32,
    _fill: usize,
) -> i32 {
    if !output(handle) {
        return fail(ERROR_INVALID_HANDLE);
    }
    if scroll == 0 {
        return fail(ERROR_INVALID_PARAMETER);
    }
    let r = |i: usize| unsafe { ((scroll + 2 * i) as *const i16).read_unaligned() };
    let s = screen();
    let (left, top, right, bottom) = (r(0), r(1).max(0), r(2), r(3).min(s.rows - 1));
    let (_, to_y) = coord(destination);
    let delta = to_y - top;
    if left > 0 || right < s.columns - 1 || bottom <= top || delta == 0 {
        return 1;
    }
    let mut out = Out::new();
    out.push(b"\x1b7\x1b[");
    out.number(top as u32 + 1);
    out.push(b";");
    out.number(bottom as u32 + 1);
    out.push(b"r\x1b[");
    out.number(delta.unsigned_abs() as u32);
    out.push(if delta < 0 { b"S" } else { b"T" });
    out.push(b"\x1b[r\x1b8");
    out.flush();
    1
}
/// The largest window the Terminal offers at its current font: twice the
/// grid in each direction.
#[unsafe(no_mangle)]
pub extern "system" fn GetLargestConsoleWindowSize(handle: usize) -> u32 {
    if !output(handle) {
        fail(ERROR_INVALID_HANDLE);
        return 0;
    }
    let s = screen();
    pack(s.columns.max(120) * 2, s.rows.max(30) * 2)
}
/// The buffer is the Terminal's grid, which the user sizes: a program's
/// request is accepted, as Windows Terminal accepts it, and the grid keeps
/// its size.
#[unsafe(no_mangle)]
pub extern "system" fn SetConsoleScreenBufferSize(handle: usize, size: u32) -> i32 {
    if !output(handle) {
        return fail(ERROR_INVALID_HANDLE);
    }
    let (columns, rows) = coord(size);
    if columns < 1 || rows < 1 {
        return fail(ERROR_INVALID_PARAMETER);
    }
    1
}
#[unsafe(no_mangle)]
pub extern "system" fn SetConsoleWindowInfo(handle: usize, _absolute: i32, window: usize) -> i32 {
    if !output(handle) {
        return fail(ERROR_INVALID_HANDLE);
    }
    if window == 0 {
        return fail(ERROR_INVALID_PARAMETER);
    }
    1
}

// ---- title and code pages ---------------------------------------------------------

#[unsafe(no_mangle)]
pub extern "system" fn GetConsoleTitleW(buffer: usize, size: u32) -> u32 {
    if buffer == 0 || size == 0 {
        fail(ERROR_INVALID_PARAMETER);
        return 0;
    }
    with(|s| {
        let n = s.title_len.min(size as usize - 1);
        for (i, unit) in s.title[..n].iter().chain([&0]).enumerate() {
            unsafe { (buffer as *mut u16).add(i).write_unaligned(*unit) };
        }
        n as u32
    })
}
#[unsafe(no_mangle)]
pub extern "system" fn GetConsoleOriginalTitleW(buffer: usize, size: u32) -> u32 {
    GetConsoleTitleW(buffer, size)
}
/// The title is kept and set on the Terminal's tab (OSC 0), as Windows
/// Terminal titles a tab from its console.
#[unsafe(no_mangle)]
pub extern "system" fn SetConsoleTitleW(title: usize) -> i32 {
    if title == 0 {
        return fail(ERROR_INVALID_PARAMETER);
    }
    let mut out = Out::new();
    out.push(b"\x1b]0;");
    with(|s| {
        let mut n = 0;
        while n < s.title.len() {
            let unit = unsafe { (title as *const u16).add(n).read_unaligned() };
            if unit == 0 {
                break;
            }
            s.title[n] = unit;
            n += 1;
        }
        s.title_len = n;
        for unit in char::decode_utf16(s.title[..n].iter().copied()) {
            let c = unit.unwrap_or('\u{fffd}');
            if !c.is_control() {
                let mut bytes = [0u8; 4];
                let text = c.encode_utf8(&mut bytes).as_bytes();
                let mut copy = [0u8; 4];
                copy[..text.len()].copy_from_slice(text);
                out.push(&copy[..text.len()]);
            }
        }
    });
    out.push(b"\x07");
    out.flush();
    1
}
/// Doors' console is UTF-8 both ways; another code page is accepted for
/// the program's own conversions (GetConsoleCP keeps answering UTF-8).
#[unsafe(no_mangle)]
pub extern "system" fn SetConsoleCP(code_page: u32) -> i32 {
    if matches!(code_page, 65001 | 437 | 850 | 1252 | 20127 | 28591) {
        1
    } else {
        fail(ERROR_INVALID_PARAMETER)
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn SetConsoleOutputCP(code_page: u32) -> i32 {
    SetConsoleCP(code_page)
}
/// No console window: the Terminal hosts the console.
#[unsafe(no_mangle)]
pub extern "system" fn GetConsoleWindow() -> usize {
    0
}

// ---- input ------------------------------------------------------------------------

/// Text from the console (ReadFile: a line edited and echoed by the
/// console with ENABLE_LINE_INPUT, else the keys typed) as UTF-16 into
/// `out`: the units, or the error.
fn read_line(out: &mut [u16; 512]) -> Result<usize, u32> {
    let mut bytes = [0u8; 1024];
    let mut got = 0u32;
    if crate::guest::ReadFile(
        abi::narrow(abi::INPUT_HANDLE),
        bytes.as_mut_ptr() as usize,
        bytes.len() as u32,
        &mut got as *mut u32 as usize,
        0,
    ) == 0
    {
        return Err(unsafe { RtlGetLastWin32Error() });
    }
    let text = core::str::from_utf8(&bytes[..got as usize]).unwrap_or("");
    let mut n = 0;
    for unit in text.encode_utf16() {
        if n == out.len() {
            break;
        }
        out[n] = unit;
        n += 1;
    }
    Ok(n)
}
/// `ReadConsoleW`: a line (with its CR LF) into the caller's buffer of
/// `characters` units; what does not fit waits for the next call.
#[unsafe(no_mangle)]
pub extern "system" fn ReadConsoleW(
    handle: usize,
    buffer: usize,
    characters: u32,
    read: usize,
    _control: usize,
) -> i32 {
    if !input(handle) {
        return fail(ERROR_INVALID_HANDLE);
    }
    if buffer == 0 {
        return fail(ERROR_INVALID_PARAMETER);
    }
    if with(|s| s.key_len) == 0 {
        let mut line = [0u16; 512];
        let n = match read_line(&mut line) {
            Ok(n) => n,
            Err(code) => return fail(code),
        };
        with(|s| {
            s.keys[..n].copy_from_slice(&line[..n]);
            s.key_start = 0;
            s.key_len = n;
        });
    }
    let n = with(|s| {
        let n = s.key_len.min(characters as usize);
        for i in 0..n {
            unsafe {
                (buffer as *mut u16)
                    .add(i)
                    .write_unaligned(s.keys[s.key_start + i])
            };
        }
        s.key_start += n;
        s.key_len -= n;
        n
    });
    write(read, n as u32);
    1
}
#[unsafe(no_mangle)]
pub extern "system" fn PeekConsoleInputW(
    handle: usize,
    records: usize,
    length: u32,
    read: usize,
) -> i32 {
    if !input(handle) {
        return fail(ERROR_INVALID_HANDLE);
    }
    unsafe { DoorConsoleReadKeys(abi::widen(records), length, abi::widen(read), 0) }
}
/// `ReadConsoleInputW`: key events, waiting for one.
#[unsafe(no_mangle)]
pub extern "system" fn ReadConsoleInputW(
    handle: usize,
    records: usize,
    length: u32,
    read: usize,
) -> i32 {
    if !input(handle) {
        return fail(ERROR_INVALID_HANDLE);
    }
    if records == 0 || length == 0 {
        return fail(ERROR_INVALID_PARAMETER);
    }
    unsafe { DoorConsoleReadKeys(abi::widen(records), length, abi::widen(read), 3) }
}
#[unsafe(no_mangle)]
pub extern "system" fn GetNumberOfConsoleInputEvents(handle: usize, count: usize) -> i32 {
    if !input(handle) {
        return fail(ERROR_INVALID_HANDLE);
    }
    unsafe { DoorConsoleInputEvents(abi::widen(count)) }
}
#[unsafe(no_mangle)]
pub extern "system" fn FlushConsoleInputBuffer(handle: usize) -> i32 {
    if !input(handle) {
        return fail(ERROR_INVALID_HANDLE);
    }
    with(|s| {
        s.key_start = 0;
        s.key_len = 0;
    });
    unsafe { DoorConsoleFlushInput() }
}

// ---- console allocation, fonts and injected input ------------------------------

/// The process already has the session's console (the Terminal's).
#[unsafe(no_mangle)]
pub extern "system" fn AllocConsole() -> i32 {
    fail(5)
}
/// `FreeConsole`: the process keeps writing to the session's console
/// stream; detaching succeeds.
#[unsafe(no_mangle)]
pub extern "system" fn FreeConsole() -> i32 {
    1
}
#[unsafe(no_mangle)]
pub extern "system" fn AttachConsole(_process: u32) -> i32 {
    fail(5)
}
/// CONSOLE_FONT_INFOEX (84 bytes): Windows Terminal's default font,
/// Cascadia Mono, in an 8 x 16 cell.
#[unsafe(no_mangle)]
pub extern "system" fn GetCurrentConsoleFontEx(handle: usize, _maximum: i32, info: usize) -> i32 {
    if !output(handle) {
        return fail(ERROR_INVALID_HANDLE);
    }
    if info == 0 || unsafe { (info as *const u32).read_unaligned() } != 84 {
        return fail(ERROR_INVALID_PARAMETER);
    }
    let mut out = [0u8; 84];
    out[..4].copy_from_slice(&84u32.to_le_bytes());
    out[8..10].copy_from_slice(&8i16.to_le_bytes());
    out[10..12].copy_from_slice(&16i16.to_le_bytes());
    // FF_MODERN | TMPF_VECTOR | TMPF_TRUETYPE, FW_NORMAL.
    out[12..16].copy_from_slice(&0x36u32.to_le_bytes());
    out[16..20].copy_from_slice(&400u32.to_le_bytes());
    for (i, unit) in "Cascadia Mono".encode_utf16().enumerate() {
        out[20 + 2 * i..22 + 2 * i].copy_from_slice(&unit.to_le_bytes());
    }
    write(info, out);
    1
}
#[unsafe(no_mangle)]
pub extern "system" fn SetCurrentConsoleFontEx(handle: usize, _maximum: i32, _info: usize) -> i32 {
    if !output(handle) {
        return fail(ERROR_INVALID_HANDLE);
    }
    1
}
/// `WriteConsoleInputW`: key events join the input this process reads
/// next.
#[unsafe(no_mangle)]
pub extern "system" fn WriteConsoleInputW(
    handle: usize,
    records: usize,
    length: u32,
    written: usize,
) -> i32 {
    if !input(handle) {
        return fail(ERROR_INVALID_HANDLE);
    }
    if records == 0 && length != 0 {
        return fail(ERROR_INVALID_PARAMETER);
    }
    if unsafe { DoorConsoleWriteKeys(abi::widen(records), length) } == 0 {
        return 0;
    }
    write(written, length);
    1
}
/// `GetConsoleProcessList(list, count)`: the processes attached to the
/// console this process knows of (itself); the count, which may exceed
/// `count` (then nothing is written).
#[unsafe(no_mangle)]
pub extern "system" fn GetConsoleProcessList(list: usize, count: u32) -> u32 {
    if list == 0 || count == 0 {
        unsafe { RtlSetLastWin32Error(ERROR_INVALID_PARAMETER) };
        return 0;
    }
    write(list, crate::application::GetCurrentProcessId());
    1
}
