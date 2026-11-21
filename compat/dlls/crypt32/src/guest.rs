//! crypt32.dll exports: certificate stores and contexts, names, signature
//! checks and data protection.
use crate::{
    CRYPT_E_ASN1_BADTAG, CRYPT_E_EXISTS, CRYPT_E_INVALID_MSG_TYPE, CRYPT_E_NO_KEY_PROPERTY,
    CRYPT_E_NO_MATCH, CRYPT_E_NOT_FOUND, E_ACCESSDENIED, E_INVALIDARG, ERROR_INVALID_DATA,
    ERROR_INVALID_PARAMETER, ERROR_MORE_DATA, ERROR_NOT_ENOUGH_MEMORY, NTE_BAD_ALGID,
    NTE_BAD_SIGNATURE, cert, dpapi,
};
use alloc::{boxed::Box, string::String, vec::Vec};
use core::sync::atomic::{AtomicBool, Ordering};
use door_windows_abi::{narrow, widen};

mod protect;

#[global_allocator]
static HEAP: door_windows_abi::allocator::ProcessHeap = door_windows_abi::allocator::ProcessHeap;

#[link(name = "ntdll", kind = "dylib")]
unsafe extern "system" {
    fn RtlSetLastWin32Error(code: u32);
    fn RtlAllocateHeap(heap: usize, flags: u32, bytes: usize) -> usize;
    fn RtlFreeHeap(heap: usize, flags: u32, pointer: usize) -> u8;
    fn DoorGetProcessHeap() -> u64;
    fn NtQuerySystemTime(time: usize) -> u32;
}
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

const WIDTH: usize = core::mem::size_of::<usize>();
const SHAPE: cert::Shape = cert::shape(WIDTH);

pub(crate) fn fail(code: u32) -> i32 {
    unsafe { RtlSetLastWin32Error(code) };
    0
}
fn set_error(code: u32) {
    unsafe { RtlSetLastWin32Error(code) };
}

// ---- guest memory --------------------------------------------------------------
fn read_u32(p: u64) -> u32 {
    unsafe { (p as *const u32).read_unaligned() }
}
fn read_ptr(p: u64) -> u64 {
    unsafe { (p as *const usize).read_unaligned() as u64 }
}
fn write_u32(p: u64, v: u32) {
    unsafe { (p as *mut u32).write_unaligned(v) }
}
fn write_ptr(p: u64, v: u64) {
    unsafe { (p as *mut usize).write_unaligned(v as usize) }
}
/// `n` bytes at `p` (empty for a null pointer).
fn bytes<'a>(p: u64, n: usize) -> &'a [u8] {
    if p == 0 || n == 0 {
        &[]
    } else {
        unsafe { core::slice::from_raw_parts(p as *const u8, n) }
    }
}
/// The bytes of a CRYPT_*_BLOB { cbData, pbData } at `p`.
pub(crate) fn blob<'a>(p: u64) -> &'a [u8] {
    if p == 0 {
        return &[];
    }
    bytes(read_ptr(p + WIDTH as u64), read_u32(p) as usize)
}
/// A NUL-terminated byte string (at most `max` bytes).
fn c_string<'a>(p: u64, max: usize) -> &'a [u8] {
    if p == 0 {
        return &[];
    }
    let mut n = 0;
    while n < max && unsafe { (p as *const u8).add(n).read() } != 0 {
        n += 1;
    }
    bytes(p, n)
}
/// A NUL-terminated UTF-16 string (at most `max` units).
pub(crate) fn wide_string(p: u64, max: usize) -> Vec<u16> {
    let mut out = Vec::new();
    if p == 0 {
        return out;
    }
    while out.len() < max {
        let unit = unsafe { (p as *const u16).add(out.len()).read_unaligned() };
        if unit == 0 {
            break;
        }
        out.push(unit);
    }
    out
}
/// A LocalAlloc-compatible block (the process heap), for buffers the
/// caller frees with LocalFree.
pub(crate) fn local_copy(data: &[u8]) -> u64 {
    let block = unsafe { RtlAllocateHeap(narrow(DoorGetProcessHeap()), 0, data.len().max(1)) };
    if block != 0 {
        unsafe { core::ptr::copy_nonoverlapping(data.as_ptr(), block as *mut u8, data.len()) };
    }
    widen(block)
}
/// The caller-sized output protocol: `data` null asks for the size; a
/// short buffer gets ERROR_MORE_DATA and the size.
fn sized_output(data: u64, size: u64, value: &[u8]) -> i32 {
    if size == 0 {
        return fail(E_INVALIDARG);
    }
    let have = read_u32(size) as usize;
    write_u32(size, value.len() as u32);
    if data == 0 {
        return 1;
    }
    if have < value.len() {
        return fail(ERROR_MORE_DATA);
    }
    unsafe { core::ptr::copy_nonoverlapping(value.as_ptr(), data as *mut u8, value.len()) };
    1
}
/// The current time as a FILETIME.
fn now() -> u64 {
    let mut time = 0u64;
    unsafe { NtQuerySystemTime(narrow(&mut time as *mut u64 as u64)) };
    time
}
/// The bytes of a file (at most 16 MiB).
pub(crate) fn read_file(path: u64) -> Result<Vec<u8>, u32> {
    const LIMIT: u64 = 16 << 20;
    let file = unsafe { CreateFileW(narrow(path), 0x8000_0000, 3, 0, 3, 0, 0) };
    if file == usize::MAX {
        return Err(CRYPT_E_NO_MATCH);
    }
    let mut size = 0u64;
    let mut out = Vec::new();
    let ok = unsafe { GetFileSizeEx(file, &mut size as *mut u64 as usize) } != 0 && size <= LIMIT;
    if ok {
        out.resize(size as usize, 0);
        let mut read = 0u32;
        let read_ok = unsafe {
            ReadFile(
                file,
                out.as_mut_ptr() as usize,
                size as u32,
                &mut read as *mut u32 as usize,
                0,
            )
        } != 0;
        out.truncate(if read_ok { read as usize } else { 0 });
    }
    unsafe { CloseHandle(file) };
    if ok { Ok(out) } else { Err(CRYPT_E_NO_MATCH) }
}

// ---- the store lock ---------------------------------------------------------------
static HELD: AtomicBool = AtomicBool::new(false);
/// Runs `f` holding the lock that guards every store and context count.
fn locked<R>(f: impl FnOnce() -> R) -> R {
    while HELD.swap(true, Ordering::Acquire) {
        core::hint::spin_loop();
    }
    let r = f();
    HELD.store(false, Ordering::Release);
    r
}

// ---- certificate contexts ----------------------------------------------------------
/// The hidden header before each CERT_CONTEXT crypt32 creates.
#[repr(C)]
struct Header {
    magic: u32,
    references: u32,
    size: u64,
    store: u64,
    reserved: u64,
}
const HEADER: u64 = core::mem::size_of::<Header>() as u64;
const CONTEXT_MAGIC: u32 = u32::from_le_bytes(*b"CERT");
fn header(context: u64) -> *mut Header {
    (context - HEADER) as *mut Header
}
fn is_context(context: u64) -> bool {
    context > HEADER && unsafe { (*header(context)).magic } == CONTEXT_MAGIC
}
/// A context's encoded certificate.
fn encoded<'a>(context: u64) -> &'a [u8] {
    bytes(
        read_ptr(context + SHAPE.encoded as u64),
        read_u32(context + SHAPE.encoded_length as u64) as usize,
    )
}
fn info(context: u64) -> u64 {
    read_ptr(context + SHAPE.info_pointer as u64)
}
/// A new context for `der` belonging to `store` (one reference, held by
/// the store, or by the caller when `store` is 0). Needs the lock.
fn create_context(der: &[u8], store: u64) -> Result<u64, u32> {
    let size = cert::context_block(der, 0, WIDTH, store)
        .ok_or(CRYPT_E_ASN1_BADTAG)?
        .len() as u64;
    let block =
        widen(unsafe { RtlAllocateHeap(narrow(DoorGetProcessHeap()), 0, narrow(HEADER + size)) });
    if block == 0 {
        return Err(ERROR_NOT_ENOUGH_MEMORY);
    }
    let context = block + HEADER;
    let image = cert::context_block(der, context, WIDTH, store).ok_or(CRYPT_E_ASN1_BADTAG)?;
    unsafe {
        header(context).write(Header {
            magic: CONTEXT_MAGIC,
            references: 1,
            size: HEADER + size,
            store,
            reserved: 0,
        });
        core::ptr::copy_nonoverlapping(image.as_ptr(), context as *mut u8, image.len());
    }
    if store != 0 {
        unsafe { (*(store as *mut Store)).total += 1 };
    }
    Ok(context)
}
fn add_reference(context: u64) -> u64 {
    if is_context(context) {
        unsafe { (*header(context)).references += 1 };
    }
    context
}
/// Drops one reference; the last frees the context. Needs the lock.
fn release(context: u64) {
    if !is_context(context) {
        return;
    }
    let h = header(context);
    unsafe {
        (*h).references -= 1;
        if (*h).references != 0 {
            return;
        }
        let store = (*h).store;
        (*h).magic = 0;
        RtlFreeHeap(narrow(DoorGetProcessHeap()), 0, narrow(context - HEADER));
        if store != 0 {
            store_unreference(store);
        }
    }
}

// ---- stores -----------------------------------------------------------------------
/// A store: its handles plus contexts that name it keep it alive.
struct Store {
    magic: u32,
    handles: u32,
    total: u32,
    collection: bool,
    certificates: Vec<u64>,
    siblings: Vec<u64>,
}
const STORE_MAGIC: u32 = u32::from_le_bytes(*b"STOR");
fn store<'a>(handle: u64) -> Option<&'a mut Store> {
    if handle == 0 {
        return None;
    }
    let s = unsafe { &mut *(handle as *mut Store) };
    (s.magic == STORE_MAGIC).then_some(s)
}
fn new_store(collection: bool) -> u64 {
    Box::into_raw(Box::new(Store {
        magic: STORE_MAGIC,
        handles: 1,
        total: 1,
        collection,
        certificates: Vec::new(),
        siblings: Vec::new(),
    })) as u64
}
fn store_unreference(handle: u64) {
    let Some(s) = store(handle) else { return };
    s.total -= 1;
    if s.total == 0 {
        s.magic = 0;
        drop(unsafe { Box::from_raw(handle as *mut Store) });
    }
}
/// Closes one handle; the last releases the store's certificates and
/// siblings. Needs the lock.
fn close(handle: u64) -> bool {
    let Some(s) = store(handle) else { return false };
    s.handles -= 1;
    if s.handles == 0 {
        let certificates = core::mem::take(&mut s.certificates);
        let siblings = core::mem::take(&mut s.siblings);
        for c in certificates {
            release(c);
        }
        for sibling in siblings {
            close(sibling);
        }
    }
    store_unreference(handle);
    true
}
/// Every certificate a store holds, a collection's through its siblings.
fn certificates(handle: u64, depth: u32, out: &mut Vec<u64>) {
    let Some(s) = store(handle) else { return };
    out.extend_from_slice(&s.certificates);
    if depth < 8 {
        for sibling in s.siblings.clone() {
            certificates(sibling, depth + 1, out);
        }
    }
}
/// The store additions go to: a collection's first sibling.
fn target(handle: u64) -> Result<u64, u32> {
    let s = store(handle).ok_or(E_INVALIDARG)?;
    if s.collection {
        s.siblings.first().copied().ok_or(E_ACCESSDENIED)
    } else {
        Ok(handle)
    }
}
/// The system stores opened in this process, by location and name: each
/// keeps an internal handle, so what a program adds stays for the process.
static SYSTEM: Locked<Vec<(Vec<u16>, u64)>> = Locked(core::cell::UnsafeCell::new(Vec::new()));
struct Locked<T>(core::cell::UnsafeCell<T>);
unsafe impl<T> Sync for Locked<T> {}
fn system_store(location: u32, name: &[u16]) -> u64 {
    let mut key: Vec<u16> = Vec::with_capacity(name.len() + 2);
    key.push(u16::from(location & 0x00FF_0000 == 0x0002_0000));
    key.extend(name.iter().map(|u| {
        if (u16::from(b'a')..=u16::from(b'z')).contains(u) {
            u - 32
        } else {
            *u
        }
    }));
    locked(|| {
        let list = unsafe { &mut *SYSTEM.0.get() };
        if let Some((_, handle)) = list.iter().find(|(k, _)| *k == key) {
            let s = store(*handle).unwrap();
            s.handles += 1;
            s.total += 1;
            return *handle;
        }
        let handle = new_store(false);
        let s = store(handle).unwrap();
        s.handles += 1;
        s.total += 1;
        list.push((key, handle));
        handle
    })
}
/// Adds `der` to `handle` with a CERT_STORE_ADD_* disposition; the
/// store's context (with a reference for the caller when `want`).
fn add_encoded(handle: u64, der: &[u8], disposition: u32, want: bool) -> Result<u64, u32> {
    cert::parse(der).ok_or(CRYPT_E_ASN1_BADTAG)?;
    locked(|| {
        if handle == 0 {
            // No store: a context of the caller's own.
            return create_context(der, 0);
        }
        let target = target(handle)?;
        let s = store(target).ok_or(E_INVALIDARG)?;
        let existing = s.certificates.iter().position(|c| encoded(*c) == der);
        let context = match (disposition, existing) {
            // CERT_STORE_ADD_NEW.
            (1, Some(_)) => return Err(CRYPT_E_EXISTS),
            // USE_EXISTING, and NEWER(_INHERIT) for an equal certificate.
            (2 | 6 | 7, Some(i)) => s.certificates[i],
            // REPLACE_EXISTING(_INHERIT_PROPERTIES).
            (3 | 5, Some(i)) => {
                let context = create_context(der, target)?;
                let old = core::mem::replace(&mut s.certificates[i], context);
                release(old);
                context
            }
            (1..=7, _) => {
                let context = create_context(der, target)?;
                s.certificates.push(context);
                context
            }
            _ => return Err(E_INVALIDARG),
        };
        Ok(if want { add_reference(context) } else { 0 })
    })
}

// ---- CertOpenStore and friends --------------------------------------------------------
const CERT_STORE_PROV_MEMORY: u64 = 2;
const CERT_STORE_PROV_PKCS7: u64 = 5;
const CERT_STORE_PROV_FILENAME_A: u64 = 7;
const CERT_STORE_PROV_FILENAME_W: u64 = 8;
const CERT_STORE_PROV_SYSTEM_A: u64 = 9;
const CERT_STORE_PROV_SYSTEM_W: u64 = 10;
const CERT_STORE_PROV_COLLECTION: u64 = 11;
const CERT_STORE_PROV_SYSTEM_REGISTRY_A: u64 = 12;
const CERT_STORE_PROV_SYSTEM_REGISTRY_W: u64 = 13;
/// A memory store holding the certificates of `data` (DER, PEM or PKCS
/// #7).
fn store_with(data: &[u8]) -> Result<u64, u32> {
    let found = certificates_in(data).ok_or(CRYPT_E_NO_MATCH)?;
    let handle = locked(|| new_store(false));
    for der in &found.2 {
        add_encoded(handle, der, 4, false)?;
    }
    Ok(handle)
}
/// The certificates of `data` and its CERT_QUERY content and format
/// types: (content, format, certificates).
fn certificates_in(data: &[u8]) -> Option<(u32, u32, Vec<Vec<u8>>)> {
    if cert::parse(data).is_some() {
        return Some((1, 1, alloc::vec![data.to_vec()]));
    }
    if let Some(list) = cert::pkcs7_certificates(data) {
        return Some((8, 1, list.into_iter().map(<[u8]>::to_vec).collect()));
    }
    let pem = cert::pem_certificates(data);
    if !pem.is_empty() {
        return Some((1, 2, pem));
    }
    // Base64 of DER without the PEM armour.
    let der = cert::base64(data)?;
    cert::parse(&der)
        .is_some()
        .then(|| (1, 2, alloc::vec![der]))
}
#[unsafe(no_mangle)]
pub extern "system" fn CertOpenStore(
    provider: usize,
    _encoding: u32,
    _crypt_prov: usize,
    flags: u32,
    para: usize,
) -> usize {
    let provider = widen(provider);
    let para = widen(para);
    // Named providers ("Memory", "System", "Collection", ...).
    let kind = if provider > 0xFFFF {
        let name = c_string(provider, 32);
        match () {
            _ if name.eq_ignore_ascii_case(b"Collection") => CERT_STORE_PROV_COLLECTION,
            _ if name.eq_ignore_ascii_case(b"System")
                || name.eq_ignore_ascii_case(b"SystemRegistry")
                || name.eq_ignore_ascii_case(b"Physical") =>
            {
                CERT_STORE_PROV_SYSTEM_W
            }
            _ if name.eq_ignore_ascii_case(b"PKCS7") => CERT_STORE_PROV_PKCS7,
            _ if name.eq_ignore_ascii_case(b"File") => CERT_STORE_PROV_FILENAME_W,
            _ => CERT_STORE_PROV_MEMORY,
        }
    } else {
        provider
    };
    let result = match kind {
        CERT_STORE_PROV_COLLECTION => Ok(locked(|| new_store(true))),
        CERT_STORE_PROV_SYSTEM_W | CERT_STORE_PROV_SYSTEM_REGISTRY_W => {
            Ok(system_store(flags, &wide_string(para, 260)))
        }
        CERT_STORE_PROV_SYSTEM_A | CERT_STORE_PROV_SYSTEM_REGISTRY_A => {
            let name: Vec<u16> = c_string(para, 260).iter().map(|b| u16::from(*b)).collect();
            Ok(system_store(flags, &name))
        }
        CERT_STORE_PROV_PKCS7 => store_with(blob(para)),
        CERT_STORE_PROV_FILENAME_W => read_file(para).and_then(|d| store_with(&d)),
        CERT_STORE_PROV_FILENAME_A => {
            let mut path: Vec<u16> = c_string(para, 260).iter().map(|b| u16::from(*b)).collect();
            path.push(0);
            read_file(path.as_ptr() as u64).and_then(|d| store_with(&d))
        }
        _ => Ok(locked(|| new_store(false))),
    };
    match result {
        Ok(handle) => narrow(handle),
        Err(e) => {
            set_error(e);
            0
        }
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn CertOpenSystemStoreW(_prov: usize, name: usize) -> usize {
    narrow(system_store(0x0001_0000, &wide_string(widen(name), 260)))
}
#[unsafe(no_mangle)]
pub extern "system" fn CertOpenSystemStoreA(_prov: usize, name: usize) -> usize {
    let name: Vec<u16> = c_string(widen(name), 260)
        .iter()
        .map(|b| u16::from(*b))
        .collect();
    narrow(system_store(0x0001_0000, &name))
}
#[unsafe(no_mangle)]
pub extern "system" fn CertDuplicateStore(handle: usize) -> usize {
    locked(|| {
        if let Some(s) = store(widen(handle)) {
            s.handles += 1;
            s.total += 1;
        }
    });
    handle
}
#[unsafe(no_mangle)]
pub extern "system" fn CertCloseStore(handle: usize, _flags: u32) -> i32 {
    if handle == 0 {
        return 1;
    }
    if locked(|| close(widen(handle))) {
        1
    } else {
        fail(E_INVALIDARG)
    }
}
/// CertControlStore: stores live in memory, so resynchronizing, commits
/// and change notifications (which never fire) all succeed.
#[unsafe(no_mangle)]
pub extern "system" fn CertControlStore(
    handle: usize,
    _flags: u32,
    control: u32,
    _para: usize,
) -> i32 {
    if locked(|| store(widen(handle)).is_none()) {
        return fail(E_INVALIDARG);
    }
    if (1..=5).contains(&control) {
        1
    } else {
        fail(ERROR_INVALID_PARAMETER)
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn CertAddStoreToCollection(
    collection: usize,
    sibling: usize,
    _flags: u32,
    priority: u32,
) -> i32 {
    let (collection, sibling) = (widen(collection), widen(sibling));
    let added = locked(|| {
        if store(sibling).is_none() {
            return false;
        }
        let Some(c) = store(collection).filter(|c| c.collection) else {
            return false;
        };
        let s = store(sibling).unwrap();
        s.handles += 1;
        s.total += 1;
        if priority > 0 {
            c.siblings.insert(0, sibling);
        } else {
            c.siblings.push(sibling);
        }
        true
    });
    if added { 1 } else { fail(E_INVALIDARG) }
}
#[unsafe(no_mangle)]
pub extern "system" fn CertRemoveStoreFromCollection(collection: usize, sibling: usize) {
    let (collection, sibling) = (widen(collection), widen(sibling));
    locked(|| {
        if let Some(c) = store(collection)
            && let Some(i) = c.siblings.iter().position(|s| *s == sibling)
        {
            c.siblings.remove(i);
            close(sibling);
        }
    });
}

// ---- adding, enumerating and finding certificates -------------------------------------
#[unsafe(no_mangle)]
pub extern "system" fn CertAddEncodedCertificateToStore(
    handle: usize,
    _encoding: u32,
    data: usize,
    length: u32,
    disposition: u32,
    out: usize,
) -> i32 {
    let out = widen(out);
    let der = bytes(widen(data), length as usize);
    match add_encoded(widen(handle), der, disposition, out != 0 || handle == 0) {
        Ok(context) => {
            if out != 0 {
                write_ptr(out, context);
            } else if handle == 0 {
                locked(|| release(context));
            }
            1
        }
        Err(e) => {
            if out != 0 {
                write_ptr(out, 0);
            }
            fail(e)
        }
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn CertAddCertificateContextToStore(
    handle: usize,
    context: usize,
    disposition: u32,
    out: usize,
) -> i32 {
    let context = widen(context);
    if context == 0 {
        return fail(E_INVALIDARG);
    }
    let der = encoded(context).to_vec();
    CertAddEncodedCertificateToStore(
        handle,
        cert::ENCODING,
        der.as_ptr() as usize,
        der.len() as u32,
        disposition,
        out,
    )
}
#[unsafe(no_mangle)]
pub extern "system" fn CertCreateCertificateContext(
    _encoding: u32,
    data: usize,
    length: u32,
) -> usize {
    let der = bytes(widen(data), length as usize);
    if cert::parse(der).is_none() {
        set_error(CRYPT_E_ASN1_BADTAG);
        return 0;
    }
    match locked(|| create_context(der, 0)) {
        Ok(context) => narrow(context),
        Err(e) => {
            set_error(e);
            0
        }
    }
}
/// The context after `previous` among a store's certificates that
/// `matches`; `previous` is released (CertEnum/CertFind semantics).
fn next_matching(handle: u64, previous: u64, matches: impl Fn(u64) -> bool) -> u64 {
    let found = locked(|| {
        let mut list = Vec::new();
        certificates(handle, 0, &mut list);
        let start = if previous == 0 {
            0
        } else {
            match list.iter().position(|c| *c == previous) {
                Some(i) => i + 1,
                None => list.len(),
            }
        };
        let found = list[start..]
            .iter()
            .copied()
            .find(|c| matches(*c))
            .map(add_reference);
        if previous != 0 {
            release(previous);
        }
        found
    });
    found.unwrap_or_else(|| {
        set_error(CRYPT_E_NOT_FOUND);
        0
    })
}
#[unsafe(no_mangle)]
pub extern "system" fn CertEnumCertificatesInStore(handle: usize, previous: usize) -> usize {
    narrow(next_matching(widen(handle), widen(previous), |_| true))
}
fn sha1(data: &[u8]) -> [u8; 20] {
    use sha1::Digest;
    sha1::Sha1::digest(data).into()
}
fn sha256(data: &[u8]) -> [u8; 32] {
    use sha2::Digest;
    sha2::Sha256::digest(data).into()
}
/// A CERT_INFO's issuer, subject and serial (big-endian) blobs.
fn info_blob<'a>(info: u64, offset: usize) -> &'a [u8] {
    blob(info + offset as u64)
}
#[unsafe(no_mangle)]
pub extern "system" fn CertFindCertificateInStore(
    handle: usize,
    _encoding: u32,
    _flags: u32,
    find: u32,
    para: usize,
    previous: usize,
) -> usize {
    let para = widen(para);
    let (handle, previous) = (widen(handle), widen(previous));
    let compare = find >> 16;
    let field = find & 0xFFFF;
    let name_offset = if field == 4 {
        SHAPE.issuer
    } else {
        SHAPE.subject
    };
    let found = match compare {
        // CERT_COMPARE_ANY.
        0 => next_matching(handle, previous, |_| true),
        // SHA1_HASH (CRYPT_HASH_BLOB*).
        1 => {
            let want = blob(para).to_vec();
            next_matching(handle, previous, |c| sha1(encoded(c))[..] == want[..])
        }
        // SHA256_HASH.
        21 => {
            let want = blob(para).to_vec();
            next_matching(handle, previous, |c| sha256(encoded(c))[..] == want[..])
        }
        // NAME (CERT_NAME_BLOB*): subject (7) or issuer (4).
        2 => {
            let want = blob(para).to_vec();
            next_matching(handle, previous, |c| {
                info_blob(info(c), name_offset) == want
            })
        }
        // NAME_STR_W / NAME_STR_A: a case-insensitive substring of the
        // rendered name.
        7 | 8 => {
            let want = if compare == 8 {
                String::from_utf16_lossy(&wide_string(para, 1024))
            } else {
                String::from_utf8_lossy(c_string(para, 1024)).into_owned()
            }
            .to_lowercase();
            next_matching(handle, previous, |c| {
                cert::x500_text(info_blob(info(c), name_offset))
                    .to_lowercase()
                    .contains(&want)
            })
        }
        // PUBLIC_KEY (CERT_PUBLIC_KEY_INFO*).
        6 => {
            let key = blob(para + 3 * WIDTH as u64).to_vec();
            next_matching(handle, previous, |c| {
                blob(info(c) + SHAPE.public_key_info as u64 + 3 * WIDTH as u64) == key
            })
        }
        // SUBJECT_CERT (CERT_INFO*: issuer and serial number).
        11 => {
            let issuer = info_blob(para, SHAPE.issuer).to_vec();
            let serial = info_blob(para, SHAPE.serial).to_vec();
            next_matching(handle, previous, |c| {
                info_blob(info(c), SHAPE.issuer) == issuer
                    && info_blob(info(c), SHAPE.serial) == serial
            })
        }
        // ISSUER_OF (CERT_CONTEXT*): certificates whose subject is its
        // issuer.
        12 => {
            let issuer = info_blob(info(para), SHAPE.issuer).to_vec();
            next_matching(handle, previous, |c| {
                info_blob(info(c), SHAPE.subject) == issuer
            })
        }
        // EXISTING (CERT_CONTEXT*).
        13 => {
            let want = encoded(para).to_vec();
            next_matching(handle, previous, |c| encoded(c) == want)
        }
        _ => {
            if previous != 0 {
                locked(|| release(previous));
            }
            set_error(E_INVALIDARG);
            0
        }
    };
    narrow(found)
}
#[unsafe(no_mangle)]
pub extern "system" fn CertDuplicateCertificateContext(context: usize) -> usize {
    narrow(locked(|| add_reference(widen(context))))
}
#[unsafe(no_mangle)]
pub extern "system" fn CertFreeCertificateContext(context: usize) -> i32 {
    locked(|| release(widen(context)));
    1
}
#[unsafe(no_mangle)]
pub extern "system" fn CertDeleteCertificateFromStore(context: usize) -> i32 {
    let context = widen(context);
    locked(|| {
        if !is_context(context) {
            return;
        }
        let owner = unsafe { (*header(context)).store };
        if let Some(s) = store(owner)
            && let Some(i) = s.certificates.iter().position(|c| *c == context)
        {
            s.certificates.remove(i);
            release(context);
        }
        // The caller's reference goes too.
        release(context);
    });
    1
}

// ---- properties, names and usages --------------------------------------------------------
const CERT_SHA1_HASH_PROP_ID: u32 = 3;
const CERT_ACCESS_STATE_PROP_ID: u32 = 14;
const CERT_KEY_IDENTIFIER_PROP_ID: u32 = 20;
const CERT_SHA256_HASH_PROP_ID: u32 = 107;
#[unsafe(no_mangle)]
pub extern "system" fn CertGetCertificateContextProperty(
    context: usize,
    property: u32,
    data: usize,
    size: usize,
) -> i32 {
    let context = widen(context);
    if context == 0 {
        return fail(E_INVALIDARG);
    }
    let der = encoded(context);
    let value: Vec<u8> = match property {
        CERT_SHA1_HASH_PROP_ID => sha1(der).to_vec(),
        CERT_SHA256_HASH_PROP_ID => sha256(der).to_vec(),
        CERT_KEY_IDENTIFIER_PROP_ID => {
            let Some(parts) = cert::parse(der) else {
                return fail(CRYPT_E_NOT_FOUND);
            };
            // subjectKeyIdentifier's OCTET STRING, else SHA-1 of the key.
            parts
                .extensions
                .iter()
                .find(|e| e.oid == "2.5.29.14")
                .and_then(|e| {
                    door_tls::der::Reader::new(e.value).expect(door_tls::der::OCTET_STRING)
                })
                .map_or_else(|| sha1(parts.key).to_vec(), |o| o.value.to_vec())
        }
        CERT_ACCESS_STATE_PROP_ID => 0u32.to_le_bytes().to_vec(),
        _ => return fail(CRYPT_E_NOT_FOUND),
    };
    sized_output(widen(data), widen(size), &value)
}
/// CertGetNameString's text for `kind` (CERT_NAME_*_TYPE).
fn name_text(context: u64, kind: u32, flags: u32, para: u64) -> String {
    let issuer = flags & 1 != 0;
    let name = info_blob(
        info(context),
        if issuer { SHAPE.issuer } else { SHAPE.subject },
    );
    let parts = cert::parse(encoded(context));
    match kind {
        // EMAIL: rfc822Name, else the E attribute.
        1 => parts
            .as_ref()
            .filter(|_| !issuer)
            .and_then(|p| {
                cert::alt_names(p, 1)
                    .first()
                    .map(|n| String::from_utf8_lossy(n).into_owned())
            })
            .or_else(|| cert::attribute(name, cert::OID_EMAIL))
            .unwrap_or_default(),
        // RDN.
        2 => cert::x500_text(name),
        // ATTR: the attribute whose OID `para` names.
        3 => {
            let oid = String::from_utf8_lossy(c_string(para, 64)).into_owned();
            cert::attribute(name, &oid).unwrap_or_default()
        }
        // SIMPLE_DISPLAY, FRIENDLY_DISPLAY (no friendly names are set).
        4 | 5 => cert::simple_display(name),
        // DNS: dNSName entries, else the common name.
        6 => parts
            .as_ref()
            .filter(|_| !issuer)
            .and_then(|p| {
                cert::alt_names(p, 2)
                    .first()
                    .map(|n| String::from_utf8_lossy(n).into_owned())
            })
            .or_else(|| cert::attribute(name, cert::OID_COMMON_NAME))
            .unwrap_or_default(),
        _ => String::new(),
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn CertGetNameStringW(
    context: usize,
    kind: u32,
    flags: u32,
    para: usize,
    name: usize,
    units: u32,
) -> u32 {
    let (context, name) = (widen(context), widen(name));
    let text: Vec<u16> = if context == 0 {
        Vec::new()
    } else {
        name_text(context, kind, flags, widen(para))
            .encode_utf16()
            .collect()
    };
    if name == 0 || units == 0 {
        return text.len() as u32 + 1;
    }
    let n = text.len().min(units as usize - 1);
    unsafe {
        core::ptr::copy_nonoverlapping(text.as_ptr(), name as *mut u16, n);
        (name as *mut u16).add(n).write_unaligned(0);
    }
    n as u32 + 1
}
#[unsafe(no_mangle)]
pub extern "system" fn CertGetNameStringA(
    context: usize,
    kind: u32,
    flags: u32,
    para: usize,
    name: usize,
    length: u32,
) -> u32 {
    let (context, name) = (widen(context), widen(name));
    let text: Vec<u8> = if context == 0 {
        Vec::new()
    } else {
        name_text(context, kind, flags, widen(para))
            .chars()
            .map(|c| if c.is_ascii() { c as u8 } else { b'?' })
            .collect()
    };
    if name == 0 || length == 0 {
        return text.len() as u32 + 1;
    }
    let n = text.len().min(length as usize - 1);
    unsafe {
        core::ptr::copy_nonoverlapping(text.as_ptr(), name as *mut u8, n);
        (name as *mut u8).add(n).write(0);
    }
    n as u32 + 1
}
/// The value of extension `oid` in a CERT_INFO.
fn info_extension<'a>(info: u64, oid: &[u8]) -> Option<&'a [u8]> {
    let count = read_u32(info + SHAPE.extension_count as u64) as u64;
    let list = read_ptr(info + SHAPE.extensions as u64);
    (0..count.min(256))
        .map(|i| list + i * SHAPE.extension as u64)
        .find(|e| c_string(read_ptr(*e), 64) == oid)
        .map(|e| blob(e + 2 * WIDTH as u64))
}
#[unsafe(no_mangle)]
pub extern "system" fn CertGetIntendedKeyUsage(
    _encoding: u32,
    info: usize,
    usage: usize,
    length: u32,
) -> i32 {
    let (info, usage) = (widen(info), widen(usage));
    if info == 0 {
        return fail(E_INVALIDARG);
    }
    let Some(bits) = info_extension(info, b"2.5.29.15").and_then(cert::key_usage) else {
        // No key usage: every use is allowed.
        return fail(0);
    };
    if usage != 0 {
        for i in 0..length as usize {
            unsafe { (usage as *mut u8).add(i).write(*bits.get(i).unwrap_or(&0)) };
        }
    }
    1
}
/// CertGetEnhancedKeyUsage: the extendedKeyUsage purposes as a
/// CERT_ENHKEY_USAGE; none (with CRYPT_E_NOT_FOUND) means every purpose.
#[unsafe(no_mangle)]
pub extern "system" fn CertGetEnhancedKeyUsage(
    context: usize,
    _flags: u32,
    usage: usize,
    size: usize,
) -> i32 {
    let (context, usage, size) = (widen(context), widen(usage), widen(size));
    if context == 0 || size == 0 {
        return fail(E_INVALIDARG);
    }
    let mut oids: Vec<String> = Vec::new();
    let mut present = false;
    if let Some(parts) = cert::parse(encoded(context))
        && let Some(e) = parts.extensions.iter().find(|e| e.oid == "2.5.29.37")
        && let Some(list) = door_tls::der::Reader::new(e.value).expect(door_tls::der::SEQUENCE)
    {
        present = true;
        let mut r = door_tls::der::Reader::new(list.value);
        while let Some(o) = r.read() {
            if let Some(text) = cert::oid_text(o.value) {
                oids.push(text);
            }
        }
    }
    // { cUsageIdentifier, rgpszUsageIdentifier }, the pointer array, then
    // the strings.
    let array = 2 * WIDTH;
    let strings = array + oids.len() * WIDTH;
    let needed = strings + oids.iter().map(|o| o.len() + 1).sum::<usize>();
    let have = read_u32(size) as usize;
    write_u32(size, needed as u32);
    if usage != 0 {
        if have < needed {
            return fail(ERROR_MORE_DATA);
        }
        write_u32(usage, oids.len() as u32);
        write_ptr(
            usage + WIDTH as u64,
            if oids.is_empty() {
                0
            } else {
                usage + array as u64
            },
        );
        let mut at = usage + strings as u64;
        for (i, o) in oids.iter().enumerate() {
            write_ptr(usage + (array + i * WIDTH) as u64, at);
            unsafe {
                core::ptr::copy_nonoverlapping(o.as_ptr(), at as *mut u8, o.len());
                (at as *mut u8).add(o.len()).write(0);
            }
            at += o.len() as u64 + 1;
        }
    }
    if !present {
        set_error(CRYPT_E_NOT_FOUND);
    }
    1
}
/// CertVerifyTimeValidity: -1 before NotBefore, 1 after NotAfter, else 0.
#[unsafe(no_mangle)]
pub extern "system" fn CertVerifyTimeValidity(time: usize, info: usize) -> i32 {
    let (time, info) = (widen(time), widen(info));
    let at = if time == 0 {
        now()
    } else {
        unsafe { (time as *const u64).read_unaligned() }
    };
    let read_filetime =
        |offset: usize| unsafe { ((info + offset as u64) as *const u64).read_unaligned() };
    if at < read_filetime(SHAPE.not_before) {
        -1
    } else if at > read_filetime(SHAPE.not_after) {
        1
    } else {
        0
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn CertCompareCertificateName(
    _encoding: u32,
    first: usize,
    second: usize,
) -> i32 {
    i32::from(blob(widen(first)) == blob(widen(second)))
}
#[unsafe(no_mangle)]
pub extern "system" fn CertCompareCertificate(_encoding: u32, first: usize, second: usize) -> i32 {
    let (a, b) = (widen(first), widen(second));
    i32::from(
        info_blob(a, SHAPE.issuer) == info_blob(b, SHAPE.issuer)
            && info_blob(a, SHAPE.serial) == info_blob(b, SHAPE.serial),
    )
}

// ---- chains, keys and messages ------------------------------------------------------------
/// `CertGetCertificateChain`: chains are not built here; callers (Chromium,
/// .NET) verify with their own engines.
#[unsafe(no_mangle)]
#[allow(clippy::too_many_arguments)]
pub extern "system" fn CertGetCertificateChain(
    _engine: usize,
    _context: usize,
    _time: usize,
    _additional: usize,
    _para: usize,
    _flags: u32,
    _reserved: usize,
    chain: usize,
) -> i32 {
    let chain = widen(chain);
    if chain != 0 {
        write_ptr(chain, 0);
    }
    fail(CRYPT_E_NOT_FOUND)
}
#[unsafe(no_mangle)]
pub extern "system" fn CertFreeCertificateChain(_chain: usize) {}
#[unsafe(no_mangle)]
pub extern "system" fn CertVerifyCertificateChainPolicy(
    _policy: usize,
    chain: usize,
    _para: usize,
    _status: usize,
) -> i32 {
    if chain == 0 {
        return fail(E_INVALIDARG);
    }
    fail(CRYPT_E_NOT_FOUND)
}
/// CertFindChainInStore: client-certificate chains need private keys,
/// which no store holds.
#[unsafe(no_mangle)]
pub extern "system" fn CertFindChainInStore(
    _store: usize,
    _encoding: u32,
    _flags: u32,
    _find: u32,
    _para: usize,
    _previous: usize,
) -> usize {
    set_error(CRYPT_E_NOT_FOUND);
    0
}
#[unsafe(no_mangle)]
pub extern "system" fn CryptAcquireCertificatePrivateKey(
    _context: usize,
    _flags: u32,
    _parameters: usize,
    key: usize,
    _spec: usize,
    _free: usize,
) -> i32 {
    let key = widen(key);
    if key != 0 {
        write_ptr(key, 0);
    }
    fail(CRYPT_E_NO_KEY_PROPERTY)
}
/// Cryptographic messages are never opened (CryptQueryObject returns
/// stores, not messages), so no handle names one.
#[unsafe(no_mangle)]
pub extern "system" fn CryptMsgClose(message: usize) -> i32 {
    if message == 0 { 1 } else { fail(E_INVALIDARG) }
}
#[unsafe(no_mangle)]
pub extern "system" fn CryptMsgGetParam(
    _message: usize,
    _kind: u32,
    _index: u32,
    _data: usize,
    _size: usize,
) -> i32 {
    fail(CRYPT_E_INVALID_MSG_TYPE)
}
/// CryptVerifyCertificateSignatureEx: an encoded certificate (blob or
/// context) checked against a public key or an issuer certificate.
#[unsafe(no_mangle)]
#[allow(clippy::too_many_arguments)]
pub extern "system" fn CryptVerifyCertificateSignatureEx(
    _prov: usize,
    _encoding: u32,
    subject_kind: u32,
    subject: usize,
    issuer_kind: u32,
    issuer: usize,
    _flags: u32,
    _extra: usize,
) -> i32 {
    let (subject, issuer) = (widen(subject), widen(issuer));
    let subject = match subject_kind {
        1 => blob(subject),
        2 => encoded(subject),
        _ => return fail(E_INVALIDARG),
    };
    let key = match issuer_kind {
        // CERT_PUBLIC_KEY_INFO { Algorithm { pszObjId, Parameters },
        // PublicKey { cbData, pbData, cUnusedBits } }.
        1 => cert::spki_from(
            c_string(read_ptr(issuer), 64),
            blob(issuer + WIDTH as u64),
            blob(issuer + 3 * WIDTH as u64),
            read_u32(issuer + 5 * WIDTH as u64),
        ),
        2 => cert::spki_contents(encoded(issuer)).map(<[u8]>::to_vec),
        _ => return fail(E_INVALIDARG),
    };
    let Some(key) = key else {
        return fail(NTE_BAD_ALGID);
    };
    match cert::verify_signature(subject, &key) {
        Ok(()) => 1,
        Err(cert::Signature::Bad) => fail(NTE_BAD_SIGNATURE),
        Err(cert::Signature::Algorithm) => fail(NTE_BAD_ALGID),
        Err(cert::Signature::Malformed) => fail(CRYPT_E_ASN1_BADTAG),
    }
}

// ---- CryptQueryObject ------------------------------------------------------------------
/// CryptQueryObject: certificates in a file or blob (DER, PEM or
/// Base64, or a PKCS #7 bundle) come back in a memory store, with the
/// first as the context; other content (signed messages, embedded
/// Authenticode signatures) does not match.
#[unsafe(no_mangle)]
#[allow(clippy::too_many_arguments)]
pub extern "system" fn CryptQueryObject(
    kind: u32,
    object: usize,
    expected_content: u32,
    expected_format: u32,
    _flags: u32,
    encoding: usize,
    content: usize,
    format: usize,
    out_store: usize,
    message: usize,
    context: usize,
) -> i32 {
    let object = widen(object);
    let data = match kind {
        1 => match read_file(object) {
            Ok(d) => d,
            Err(e) => return fail(e),
        },
        2 => blob(object).to_vec(),
        _ => return fail(E_INVALIDARG),
    };
    let Some((found_content, found_format, list)) = certificates_in(&data) else {
        return fail(CRYPT_E_NO_MATCH);
    };
    // CERT_QUERY_CONTENT_FLAG_* is 1 << CERT_QUERY_CONTENT_*.
    if expected_content & (1 << found_content) == 0 || expected_format & (1 << found_format) == 0 {
        return fail(CRYPT_E_NO_MATCH);
    }
    let handle = locked(|| new_store(false));
    let mut first = 0;
    for der in &list {
        match add_encoded(handle, der, 4, first == 0) {
            Ok(c) if first == 0 => first = c,
            Ok(_) => (),
            Err(e) => {
                locked(|| close(handle));
                return fail(e);
            }
        }
    }
    let put_u32 = |p: usize, v: u32| {
        if p != 0 {
            write_u32(widen(p), v);
        }
    };
    put_u32(encoding, cert::ENCODING);
    put_u32(content, found_content);
    put_u32(format, found_format);
    if message != 0 {
        write_ptr(widen(message), 0);
    }
    if context != 0 && found_content == 1 {
        write_ptr(widen(context), first);
    } else {
        locked(|| release(first));
        if context != 0 {
            write_ptr(widen(context), 0);
        }
    }
    if out_store != 0 {
        write_ptr(widen(out_store), handle);
    } else {
        locked(|| close(handle));
    }
    1
}

// ---- OIDs and decoding ---------------------------------------------------------------------
/// `CryptDecodeObject`: no structure decoder is installed.
#[unsafe(no_mangle)]
pub extern "system" fn CryptDecodeObject(
    _encoding: u32,
    _structure: usize,
    _encoded: usize,
    _length: u32,
    _flags: u32,
    _out: usize,
    _out_length: usize,
) -> i32 {
    fail(CRYPT_E_ASN1_BADTAG)
}
#[unsafe(no_mangle)]
#[allow(clippy::too_many_arguments)]
pub extern "system" fn CryptDecodeObjectEx(
    _encoding: u32,
    _structure: usize,
    _encoded: usize,
    _length: u32,
    _flags: u32,
    _para: usize,
    _out: usize,
    _out_length: usize,
) -> i32 {
    fail(CRYPT_E_ASN1_BADTAG)
}
/// `CertOIDToAlgId(oid)`: the ALG_ID, 0 for an unknown OID.
#[unsafe(no_mangle)]
pub extern "system" fn CertOIDToAlgId(oid: usize) -> u32 {
    crate::alg_id(c_string(widen(oid), 64))
}

/// The protection key errors CryptUnprotectData reports for a blob that
/// does not open.
pub(crate) const BAD_BLOB: u32 = ERROR_INVALID_DATA;
pub(crate) use dpapi::LOCAL_MACHINE;
