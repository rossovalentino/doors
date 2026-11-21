//! Application resources (fonts, icon masks and bitmaps) loaded at run time
//! instead of being compiled into every executable.
//!
//! Native: the kernel validates `system/app-assets.pack` from the
//! authenticated initfs once at boot and gives each desktop application one
//! inherited read-only section handle (`inherited::APP_ASSETS`) to the same
//! physical copy. `install_from_startup` maps it read-only at `ADDRESS`; the
//! mapping lives until process teardown, so lookups return `'static` bytes.
//!
//! Host (tests and painters): keys are read from the repository's `assets/`
//! tree on first use and cached, and every key must be listed in
//! `profiles/build/app-assets.txt` (the pack manifest), so an asset the pack
//! would lack fails the host tests instead of rendering blank on a guest.
use alloc::sync::Arc;
use core::sync::atomic::{AtomicPtr, AtomicUsize, Ordering};
use door_graphics_text::{FontProvider, FontRole, FontSet, TextSystem};
use door_initfs::app_assets::{Pack, key_of};

/// Read-only view of the shared pack: the top 32 MiB of the user section
/// window (`surface` keeps the 192 MiB below it for window buffers).
pub const ADDRESS: u64 = 0x7E00_0000;
pub const MAX_BYTES: u64 = door_initfs::app_assets::MAX_BYTES as u64;

static BASE: AtomicUsize = AtomicUsize::new(0);
static LENGTH: AtomicUsize = AtomicUsize::new(0);

/// Publish an already validated, process-lifetime pack (tests may install
/// one too). Returns false and keeps the previous pack if `bytes` is invalid.
pub fn install(bytes: &'static [u8]) -> bool {
    if Pack::parse(bytes).is_none() {
        return false;
    }
    LENGTH.store(0, Ordering::Release);
    BASE.store(bytes.as_ptr() as usize, Ordering::Release);
    LENGTH.store(bytes.len(), Ordering::Release);
    true
}
fn installed() -> Option<Pack<'static>> {
    let length = LENGTH.load(Ordering::Acquire);
    if length == 0 {
        return None;
    }
    let base = BASE.load(Ordering::Acquire) as *const u8;
    // SAFETY: `install` only stores the address and length of a `'static`
    // slice it validated; neither is ever freed.
    Pack::parse(unsafe { core::slice::from_raw_parts(base, length) })
}
/// Map the inherited pack section read-only and install it.
pub fn install_from_startup(
    startup: &door_abi::platform::StartupV2,
) -> Result<(), door_abi::Status> {
    use door_abi::{HandleKind, Status, platform::inherited, rights};
    let grant = startup.base.inherited[inherited::APP_ASSETS];
    if grant.kind != HandleKind::Section as u64
        || grant.rights != rights::QUERY | rights::READ | rights::MAP
    {
        return Err(Status::AccessDenied);
    }
    // SAFETY: ADDRESS..ADDRESS + MAX_BYTES is reserved for this view and no
    // Rust reference to it exists before the kernel returns; it is read-only
    // and never unmapped, so the slice below stays valid and immutable.
    let bytes = unsafe { door_sys::section_map(grant.handle, ADDRESS, rights::READ) }?;
    if bytes == 0 || bytes > MAX_BYTES {
        return Err(Status::InvalidArgument);
    }
    let view = unsafe { core::slice::from_raw_parts(ADDRESS as *const u8, bytes as usize) };
    if install(view) {
        Ok(())
    } else {
        Err(Status::InvalidArgument)
    }
}
/// The resource stored for a source-relative asset path (`asset!`). Missing
/// resources are empty; every painter treats a wrongly sized input as absent.
pub fn get(path: &str) -> &'static [u8] {
    let key = key_of(path);
    if let Some(pack) = installed() {
        return pack.get(key).unwrap_or(&[]);
    }
    host::get(key)
}
/// On-demand read-only resource `id` (`door_abi::platform::resource`, the
/// full Windows fonts): mapped by the kernel's shared section on native
/// targets, read from `assets/fonts/windows-unicode/` on the host.
pub fn resource(id: u64) -> Option<&'static [u8]> {
    #[cfg(target_os = "none")]
    return door_rt::resources::open(id).ok();
    #[cfg(not(target_os = "none"))]
    {
        let path =
            door_abi::platform::resource::PATHS.get(usize::try_from(id).ok()?.checked_sub(1)?)?;
        let name = path.rsplit('/').next()?;
        Some(host::read(&alloc::format!("fonts/windows-unicode/{name}")))
    }
}
/// Font roles the kernel serves on demand (not in the asset pack).
fn resource_of(role: FontRole) -> Option<u64> {
    use door_abi::platform::resource as r;
    Some(match role {
        FontRole::Emoji => r::EMOJI,
        FontRole::Historic => r::HISTORIC,
        FontRole::ChineseSimplified => r::CHINESE_SIMPLIFIED,
        FontRole::ChineseTraditional => r::CHINESE_TRADITIONAL,
        FontRole::Japanese => r::JAPANESE,
        FontRole::Korean => r::KOREAN,
        FontRole::Indic => r::INDIC,
        _ => return None,
    })
}
/// `FontSet` provider over the on-demand resource sections: a role's file
/// is mapped the first time text needs it, at most once per process.
pub struct ResourceFonts;
impl FontProvider for ResourceFonts {
    fn load(&self, role: FontRole) -> alloc::vec::Vec<&'static [u8]> {
        resource_of(role)
            .and_then(resource)
            .filter(|bytes| !bytes.is_empty())
            .into_iter()
            .collect()
    }
}
/// Pack faces of the Unicode text engine: Segoe UI Variable, Segoe UI
/// (regular and semibold), Segoe UI Symbol and Cascadia Mono.
pub const FONT_SET: [(&str, FontRole); 5] = [
    ("fonts/segoe-ui/SegUIVar.ttf", FontRole::UiVariable),
    ("fonts/segoe-ui/segoeui.ttf", FontRole::Ui),
    ("fonts/segoe-ui/seguisb.ttf", FontRole::Ui),
    ("fonts/windows-unicode/seguisym.ttf", FontRole::Symbol),
    ("fonts/cascadia-mono/CascadiaMono.ttf", FontRole::Monospace),
];
static TEXT: AtomicPtr<TextSystem> = AtomicPtr::new(core::ptr::null_mut());
/// The process's Unicode text engine (shaping, fallback, bidi, emoji),
/// built once from the pack plus the on-demand provider. Every `FontAtlas`
/// the runtime creates attaches it; None if the pack lacks the faces.
pub fn text_system() -> Option<Arc<TextSystem>> {
    let mut current = TEXT.load(Ordering::Acquire);
    if current.is_null() {
        let files: alloc::vec::Vec<_> = FONT_SET
            .iter()
            .map(|(key, role)| (get(key), *role))
            .filter(|(bytes, _)| !bytes.is_empty())
            .collect();
        let set = FontSet::from_files(&files)
            .ok()?
            .with_provider(alloc::boxed::Box::new(ResourceFonts));
        let fresh = Arc::into_raw(Arc::new(TextSystem::new(set))) as *mut TextSystem;
        current = match TEXT.compare_exchange(
            core::ptr::null_mut(),
            fresh,
            Ordering::AcqRel,
            Ordering::Acquire,
        ) {
            Ok(_) => fresh,
            Err(winner) => {
                // SAFETY: `fresh` came from `Arc::into_raw` above and was never published.
                drop(unsafe { Arc::from_raw(fresh) });
                winner
            }
        };
    }
    // SAFETY: TEXT holds one strong reference for the process lifetime.
    unsafe {
        Arc::increment_strong_count(current);
        Some(Arc::from_raw(current))
    }
}
#[cfg(target_os = "none")]
mod host {
    pub fn get(_key: &str) -> &'static [u8] {
        &[]
    }
}
#[cfg(not(target_os = "none"))]
mod host {
    extern crate std;
    use alloc::{boxed::Box, collections::BTreeMap, string::String, vec::Vec};
    use std::sync::Mutex;
    static CACHE: Mutex<BTreeMap<String, &'static [u8]>> = Mutex::new(BTreeMap::new());
    const MANIFEST: &str = include_str!("../../../profiles/build/app-assets.txt");
    pub fn get(key: &str) -> &'static [u8] {
        assert!(
            MANIFEST.lines().any(|line| line == key),
            "asset {key} is not in profiles/build/app-assets.txt (the runtime pack manifest)"
        );
        read(key)
    }
    /// Any repository asset, cached for the process lifetime.
    pub fn read(key: &str) -> &'static [u8] {
        let mut cache = CACHE.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(bytes) = cache.get(key) {
            return bytes;
        }
        let path = std::format!("{}/../../assets/{key}", env!("CARGO_MANIFEST_DIR"));
        let bytes: Vec<u8> = std::fs::read(&path).unwrap_or_else(|e| panic!("asset {path}: {e}"));
        let bytes: &'static [u8] = Box::leak(bytes.into_boxed_slice());
        cache.insert(key.into(), bytes);
        bytes
    }
}
#[cfg(test)]
mod tests {
    #[test]
    fn host_lookups_read_listed_assets_once_and_keep_their_address() {
        let a = super::get("../../../assets/icons/windows-shell/app-add.20.mask");
        let b = super::get("../../../../../assets/icons/windows-shell/app-add.20.mask");
        assert_eq!(a.len(), 400);
        assert_eq!(a.as_ptr(), b.as_ptr(), "stable keys for the image cache");
    }
    #[test]
    fn unicode_engine_uses_pack_faces_and_maps_on_demand_fonts_once() {
        use door_graphics_text::{Family, FontAtlas, TextStyle};
        let system = super::text_system().unwrap();
        assert!(alloc::sync::Arc::ptr_eq(
            &system,
            &super::text_system().unwrap()
        ));
        let before = system.fonts().len();
        let mut atlas = FontAtlas::windows_reference_scaled(1).unwrap();
        atlas.attach_unicode(system.clone(), Family::Ui);
        let width = atlas
            .measure("日本語 😀 Ελληνικά", TextStyle::Regular14)
            .unwrap()
            .advance_width;
        assert!(width > 60, "{width}");
        assert!(
            system.fonts().len() > before,
            "CJK and emoji faces load on first use"
        );
        assert_eq!(super::resource(0), None);
        assert_eq!(super::resource(99), None);
    }
}
