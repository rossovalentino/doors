//! Font files, Doors' local font file loader, file streams, and custom
//! collections built through a program's collection loader.
use super::*;

const IID_FILE: Guid = guid(
    0x739d886a,
    0xcef5,
    0x47dc,
    [0x87, 0x69, 0x1a, 0x8b, 0x41, 0xbe, 0xbb, 0xb0],
);
const IID_LOADER: Guid = guid(
    0x727cad4e,
    0xd6af,
    0x4c9e,
    [0x8a, 0x08, 0xd6, 0x95, 0xb1, 0x1c, 0xaa, 0x49],
);
const IID_LOCAL_LOADER: Guid = guid(
    0xb2d9f3ec,
    0xc9fe,
    0x4a11,
    [0xa2, 0xec, 0xd8, 0x62, 0x08, 0xf7, 0xc0, 0xa2],
);
const IID_STREAM: Guid = guid(
    0x6d4865fe,
    0x0ab8,
    0x4d91,
    [0x8f, 0x62, 0x5d, 0xd6, 0xbe, 0x34, 0xa3, 0xe0],
);
/// The time every local font file reports (2026-01-01, a FILETIME).
const WRITE_TIME: u64 = 0x01DC_7AC0_8B3E_8000;

#[link(name = "kernel32", kind = "dylib")]
unsafe extern "system" {
    fn CreateFileW(
        name: usize,
        access: u32,
        share: u32,
        security: usize,
        disposition: u32,
        flags: u32,
        template: usize,
    ) -> usize;
    fn GetFileSizeEx(file: usize, size: usize) -> i32;
    fn ReadFile(file: usize, buffer: usize, bytes: u32, read: usize, overlapped: usize) -> i32;
    fn CloseHandle(handle: usize) -> i32;
}

// ---- font files ---------------------------------------------------------------------------
pub(crate) struct FileObject(pub Arc<File>);
static FILE_VTBL: Vtbl<6> = Vtbl([
    query::<FileObject> as *const (),
    add_ref::<FileObject> as *const (),
    release::<FileObject> as *const (),
    file_key as *const (),
    file_loader as *const (),
    file_analyze as *const (),
]);
impl Object for FileObject {
    const IIDS: &'static [Guid] = &[IID_FILE];
    fn vtbl() -> *const () {
        &FILE_VTBL as *const _ as *const ()
    }
}
pub(crate) fn new_file(file: Arc<File>) -> usize {
    new(FileObject(file))
}
extern "system" fn file_key(this: usize, key: usize, size: usize) -> u32 {
    let f = &data::<FileObject>(this).0;
    put_ptr(key, f.key.as_ptr() as usize);
    put_u32(size, f.key.len() as u32);
    S_OK
}
extern "system" fn file_loader(this: usize, out: usize) -> u32 {
    let f = &data::<FileObject>(this).0;
    let loader = if f.loader == 0 {
        local_loader()
    } else {
        f.loader as usize
    };
    com_add_ref(loader);
    put_ptr(out, loader);
    S_OK
}
extern "system" fn file_analyze(
    this: usize,
    supported: usize,
    file_type: usize,
    face_type: usize,
    faces: usize,
) -> u32 {
    let found = font::analyze(data::<FileObject>(this).0.bytes.get());
    put_u32(supported, u32::from(found.is_some()));
    let (a, b, c) = found.unwrap_or((0, 5, 0));
    put_u32(file_type, a);
    put_u32(face_type, b);
    put_u32(faces, c);
    S_OK
}
/// The file an IDWriteFontFile pointer names: ours directly, or a
/// program's own implementation read through its loader's stream.
pub(crate) fn file_of(pointer: usize) -> Result<Arc<File>, u32> {
    if pointer == 0 {
        return Err(E_INVALIDARG);
    }
    if let Some(f) = ours::<FileObject>(pointer) {
        return Ok(f.0.clone());
    }
    type Key = extern "system" fn(usize, usize, usize) -> u32;
    type Loader = extern "system" fn(usize, usize) -> u32;
    let get_key: Key = unsafe { core::mem::transmute(method(pointer, 3)) };
    let get_loader: Loader = unsafe { core::mem::transmute(method(pointer, 4)) };
    let (mut key, mut size, mut loader) = (0usize, 0u32, 0usize);
    if get_key(
        pointer,
        &mut key as *mut usize as usize,
        &mut size as *mut u32 as usize,
    ) != S_OK
        || get_loader(pointer, &mut loader as *mut usize as usize) != S_OK
    {
        return Err(DWRITE_E_FILEFORMAT);
    }
    let file = read_custom(bytes(key, size as usize).to_vec(), loader);
    com_release(loader);
    file
}
/// Reads a whole file through `loader`'s stream for `key`.
fn read_custom(key: Vec<u8>, loader: usize) -> Result<Arc<File>, u32> {
    if loader == 0 {
        return Err(E_INVALIDARG);
    }
    // Our own loader: the file is already known by its path.
    if loader == local_loader() {
        return local_file(&key);
    }
    type Create = extern "system" fn(usize, usize, u32, usize) -> u32;
    type Size = extern "system" fn(usize, usize) -> u32;
    type Read = extern "system" fn(usize, usize, u64, u64, usize) -> u32;
    type Release = extern "system" fn(usize, usize);
    let create: Create = unsafe { core::mem::transmute(method(loader, 3)) };
    let mut stream = 0usize;
    let status = create(
        loader,
        key.as_ptr() as usize,
        key.len() as u32,
        &mut stream as *mut usize as usize,
    );
    if status != S_OK || stream == 0 {
        return Err(if status == S_OK { E_FAIL } else { status });
    }
    let size_of: Size = unsafe { core::mem::transmute(method(stream, 5)) };
    let read: Read = unsafe { core::mem::transmute(method(stream, 3)) };
    let release_fragment: Release = unsafe { core::mem::transmute(method(stream, 4)) };
    let mut size = 0u64;
    let mut result = Err(DWRITE_E_FILEFORMAT);
    if size_of(stream, &mut size as *mut u64 as usize) == S_OK && size > 0 && size < (256 << 20) {
        let (mut start, mut context) = (0usize, 0usize);
        if read(
            stream,
            &mut start as *mut usize as usize,
            0,
            size,
            &mut context as *mut usize as usize,
        ) == S_OK
            && start != 0
        {
            let owned: Arc<[u8]> = Arc::from(bytes(start, size as usize));
            release_fragment(stream, context);
            com_add_ref(loader);
            result = Ok(Arc::new(File {
                bytes: Bytes::Shared(owned),
                path: Vec::new(),
                key,
                loader: loader as u64,
            }));
        }
    }
    com_release(stream);
    result
}
pub(crate) extern "system" fn create_custom_font_file_reference(
    _this: usize,
    key: usize,
    size: u32,
    loader: usize,
    out: usize,
) -> u32 {
    put_ptr(out, 0);
    if out == 0 || loader == 0 {
        return E_INVALIDARG;
    }
    match read_custom(bytes(key, size as usize).to_vec(), loader) {
        Ok(file) => {
            put_ptr(out, new_file(file));
            S_OK
        }
        Err(e) => e,
    }
}

// ---- local files -------------------------------------------------------------------------
/// Local files opened by path (other than the system fonts), kept for the
/// process so their loader keys stay valid.
static LOCAL: Once<Arc<LocalFiles>> = Once::new();
struct LocalFiles {
    held: AtomicBool,
    files: core::cell::UnsafeCell<Vec<Arc<File>>>,
}
unsafe impl Sync for LocalFiles {}
unsafe impl Send for LocalFiles {}
fn local_files<R>(f: impl FnOnce(&mut Vec<Arc<File>>) -> R) -> R {
    let l = LOCAL.get(|| {
        Arc::new(LocalFiles {
            held: AtomicBool::new(false),
            files: core::cell::UnsafeCell::new(Vec::new()),
        })
    });
    while l.held.swap(true, Ordering::Acquire) {
        core::hint::spin_loop();
    }
    let r = f(unsafe { &mut *l.files.get() });
    l.held.store(false, Ordering::Release);
    r
}
fn same_path(a: &[u16], b: &[u16]) -> bool {
    let fold = |u: &u16| match *u {
        0x2F => 0x5C,
        c if (0x61..=0x7A).contains(&c) => c - 32,
        c => c,
    };
    a.len() == b.len() && a.iter().map(fold).eq(b.iter().map(fold))
}
/// The local file a loader key (a path) names: a system font, or a file
/// read from disk.
fn local_file(key: &[u8]) -> Result<Arc<File>, u32> {
    let path: Vec<u16> = key
        .chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .take_while(|u| *u != 0)
        .collect();
    open_path(&path)
}
fn open_path(path: &[u16]) -> Result<Arc<File>, u32> {
    if let Some(f) = system_files().iter().find(|f| same_path(&f.path, path)) {
        return Ok(f.clone());
    }
    // A system font named by file name in another folder spelling.
    let name_of = |p: &[u16]| -> Vec<u16> {
        let start = p
            .iter()
            .rposition(|u| *u == 0x5C || *u == 0x2F)
            .map_or(0, |i| i + 1);
        p[start..].to_vec()
    };
    let windows_fonts: Vec<u16> = "C:\\Windows\\Fonts\\".encode_utf16().collect();
    if path.len() > windows_fonts.len() && same_path(&path[..windows_fonts.len()], &windows_fonts) {
        let name = name_of(path);
        if let Some(f) = system_files()
            .iter()
            .find(|f| same_path(&name_of(&f.path), &name))
        {
            return Ok(f.clone());
        }
    }
    if let Some(f) = local_files(|files| files.iter().find(|f| same_path(&f.path, path)).cloned()) {
        return Ok(f);
    }
    let data = read_path(path).ok_or(DWRITE_E_FILENOTFOUND)?;
    let file = Arc::new(File {
        bytes: Bytes::Shared(Arc::from(data.as_slice())),
        path: path.to_vec(),
        key: path.iter().flat_map(|u| u.to_le_bytes()).collect(),
        loader: 0,
    });
    local_files(|files| files.push(file.clone()));
    Ok(file)
}
fn read_path(path: &[u16]) -> Option<Vec<u8>> {
    let mut z = path.to_vec();
    z.push(0);
    let file = unsafe { CreateFileW(z.as_ptr() as usize, 0x8000_0000, 7, 0, 3, 0, 0) };
    if file == usize::MAX {
        return None;
    }
    let mut size = 0u64;
    let mut out = None;
    if unsafe { GetFileSizeEx(file, &mut size as *mut u64 as usize) } != 0
        && size > 0
        && size < (256 << 20)
    {
        let mut data = alloc::vec![0u8; size as usize];
        let mut read = 0u32;
        if unsafe {
            ReadFile(
                file,
                data.as_mut_ptr() as usize,
                size as u32,
                &mut read as *mut u32 as usize,
                0,
            )
        } != 0
            && u64::from(read) == size
        {
            out = Some(data);
        }
    }
    unsafe { CloseHandle(file) };
    out
}
pub(crate) extern "system" fn create_font_file_reference(
    _this: usize,
    path: usize,
    _time: usize,
    out: usize,
) -> u32 {
    put_ptr(out, 0);
    if path == 0 || out == 0 {
        return E_INVALIDARG;
    }
    match open_path(&wide(path, 32_767)) {
        Ok(file) => {
            put_ptr(out, new_file(file));
            S_OK
        }
        Err(e) => e,
    }
}

// ---- the local font file loader -----------------------------------------------------------
pub(crate) struct LocalLoader;
static LOADER_VTBL: Vtbl<7> = Vtbl([
    static_query::<LocalLoader> as *const (),
    static_count as *const (),
    static_count as *const (),
    loader_stream as *const (),
    loader_path_length as *const (),
    loader_path as *const (),
    loader_write_time as *const (),
]);
impl Object for LocalLoader {
    const IIDS: &'static [Guid] = &[IID_LOADER, IID_LOCAL_LOADER];
    fn vtbl() -> *const () {
        &LOADER_VTBL as *const _ as *const ()
    }
}
#[repr(C)]
struct StaticLoader {
    vtbl: *const (),
}
unsafe impl Sync for StaticLoader {}
static LOADER: StaticLoader = StaticLoader {
    vtbl: &LOADER_VTBL as *const _ as *const (),
};
pub(crate) fn local_loader() -> usize {
    &LOADER as *const StaticLoader as usize
}
extern "system" fn loader_stream(_this: usize, key: usize, size: u32, out: usize) -> u32 {
    put_ptr(out, 0);
    match local_file(bytes(key, size as usize)) {
        Ok(file) => {
            put_ptr(out, new(StreamObject(file)));
            S_OK
        }
        Err(e) => e,
    }
}
fn key_units(key: usize, size: u32) -> Vec<u16> {
    bytes(key, size as usize)
        .chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .take_while(|u| *u != 0)
        .collect()
}
extern "system" fn loader_path_length(_this: usize, key: usize, size: u32, length: usize) -> u32 {
    put_u32(length, key_units(key, size).len() as u32);
    S_OK
}
extern "system" fn loader_path(
    _this: usize,
    key: usize,
    size: u32,
    out: usize,
    capacity: u32,
) -> u32 {
    let units = key_units(key, size);
    if (capacity as usize) < units.len() + 1 {
        return E_NOT_SUFFICIENT_BUFFER;
    }
    let data: Vec<u8> = units
        .iter()
        .chain(&[0])
        .flat_map(|u| u.to_le_bytes())
        .collect();
    put_bytes(out, &data);
    S_OK
}
extern "system" fn loader_write_time(_this: usize, _key: usize, _size: u32, out: usize) -> u32 {
    put_bytes(out, &WRITE_TIME.to_le_bytes());
    S_OK
}

// ---- streams ----------------------------------------------------------------------------------
pub(crate) struct StreamObject(Arc<File>);
static STREAM_VTBL: Vtbl<7> = Vtbl([
    query::<StreamObject> as *const (),
    add_ref::<StreamObject> as *const (),
    release::<StreamObject> as *const (),
    stream_read as *const (),
    stream_release_fragment as *const (),
    stream_size as *const (),
    stream_write_time as *const (),
]);
impl Object for StreamObject {
    const IIDS: &'static [Guid] = &[IID_STREAM];
    fn vtbl() -> *const () {
        &STREAM_VTBL as *const _ as *const ()
    }
}
extern "system" fn stream_read(
    this: usize,
    start: usize,
    offset: u64,
    size: u64,
    context: usize,
) -> u32 {
    let data = data::<StreamObject>(this).0.bytes.get();
    put_ptr(start, 0);
    put_ptr(context, 0);
    match offset.checked_add(size) {
        Some(end) if end <= data.len() as u64 => {
            put_ptr(start, data.as_ptr() as usize + offset as usize);
            S_OK
        }
        _ => E_FAIL,
    }
}
extern "system" fn stream_release_fragment(_this: usize, _context: usize) {}
extern "system" fn stream_size(this: usize, out: usize) -> u32 {
    put_bytes(
        out,
        &(data::<StreamObject>(this).0.bytes.get().len() as u64).to_le_bytes(),
    );
    S_OK
}
extern "system" fn stream_write_time(_this: usize, out: usize) -> u32 {
    put_bytes(out, &WRITE_TIME.to_le_bytes());
    S_OK
}

// ---- custom collections -----------------------------------------------------------------------
/// CreateCustomFontCollection: the files a program's collection loader
/// enumerates for `key`.
pub(crate) extern "system" fn create_custom_font_collection(
    this: usize,
    loader: usize,
    key: usize,
    size: u32,
    out: usize,
) -> u32 {
    put_ptr(out, 0);
    if loader == 0 || out == 0 {
        return E_INVALIDARG;
    }
    type Create = extern "system" fn(usize, usize, usize, u32, usize) -> u32;
    type Next = extern "system" fn(usize, usize) -> u32;
    type Current = extern "system" fn(usize, usize) -> u32;
    let create: Create = unsafe { core::mem::transmute(method(loader, 3)) };
    let mut enumerator = 0usize;
    let status = create(
        loader,
        this,
        key,
        size,
        &mut enumerator as *mut usize as usize,
    );
    if status != S_OK || enumerator == 0 {
        return if status == S_OK { E_FAIL } else { status };
    }
    let next: Next = unsafe { core::mem::transmute(method(enumerator, 3)) };
    let current: Current = unsafe { core::mem::transmute(method(enumerator, 4)) };
    let mut faces = Vec::new();
    loop {
        let mut more = 0i32;
        if next(enumerator, &mut more as *mut i32 as usize) != S_OK || more == 0 {
            break;
        }
        let mut file = 0usize;
        if current(enumerator, &mut file as *mut usize as usize) != S_OK || file == 0 {
            continue;
        }
        if let Ok(f) = file_of(file) {
            faces.extend(font::faces(&f));
        }
        com_release(file);
    }
    com_release(enumerator);
    put_ptr(
        out,
        fonts::new_collection(Arc::new(Collection::from_faces(faces))),
    );
    S_OK
}
