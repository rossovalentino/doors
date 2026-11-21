//! Windows structures at the build's pointer width (P17). The same DLL
//! sources build for x64 and x86, and a structure holding pointers,
//! handles or pointer-sized integers (WPARAM, LPARAM, ULONG_PTR) has other
//! offsets and another size in each: a structure is described once as its
//! field kinds, laid out as MSVC lays them out (each field at its natural
//! alignment, 8-byte integers aligned to 8 on x86 too, the size rounded to
//! the largest alignment), and read or written field by field. A pointer
//! field is read as `widen` reads a pointer argument (sign-extended on x86,
//! so -1/-3 handle values keep their meaning).

/// The pointer width of this build.
pub const POINTER: usize = core::mem::size_of::<usize>();

/// A field of a Windows structure. `Bytes(n)` is an inline array of bytes
/// (alignment 1); embedded structures are spelled as their fields.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Field {
    U8,
    U16,
    U32,
    U64,
    /// A pointer, handle, WPARAM, LPARAM, ULONG_PTR, SIZE_T.
    Pointer,
    Bytes(usize),
}
use Field::*;

impl Field {
    pub const fn bytes(self, width: usize) -> usize {
        match self {
            U8 => 1,
            U16 => 2,
            U32 => 4,
            U64 => 8,
            Pointer => width,
            Bytes(n) => n,
        }
    }
    pub const fn align(self, width: usize) -> usize {
        match self {
            Bytes(_) => 1,
            _ => self.bytes(width),
        }
    }
}

const fn round(value: usize, align: usize) -> usize {
    value.div_ceil(align) * align
}
const fn packed(align: usize, pack: usize) -> usize {
    if align < pack { align } else { pack }
}
/// The offset of field `index` of `fields` at `width`, with members aligned
/// to at most `pack` bytes (`#pragma pack(n)`; 8 is the compilers' default).
pub const fn offset_packed(fields: &[Field], index: usize, width: usize, pack: usize) -> usize {
    let mut at = 0;
    let mut i = 0;
    while i < index {
        at = round(at, packed(fields[i].align(width), pack)) + fields[i].bytes(width);
        i += 1;
    }
    round(at, packed(fields[index].align(width), pack))
}
/// The structure's size at `width` and packing `pack`.
pub const fn size_packed(fields: &[Field], width: usize, pack: usize) -> usize {
    let mut at = 0;
    let mut largest = 1;
    let mut i = 0;
    while i < fields.len() {
        let align = packed(fields[i].align(width), pack);
        if align > largest {
            largest = align;
        }
        at = round(at, align) + fields[i].bytes(width);
        i += 1;
    }
    round(at, largest)
}
/// The offset of field `index` of `fields` at `width` (default packing).
pub const fn offset_at(fields: &[Field], index: usize, width: usize) -> usize {
    offset_packed(fields, index, width, 8)
}
/// The structure's size at `width` (default packing).
pub const fn size_at(fields: &[Field], width: usize) -> usize {
    size_packed(fields, width, 8)
}
/// shellapi.h's packing (SHELLEXECUTEINFO, SHFILEOPSTRUCT,
/// NOTIFYICONDATA, ...): 1 for 32-bit programs (`pshpack1.h`), the
/// default for 64-bit ones.
pub const SHELLAPI_PACK: usize = if POINTER == 4 { 1 } else { 8 };
/// `offset_at` and `size_at` for this build.
pub const fn offset(fields: &[Field], index: usize) -> usize {
    offset_at(fields, index, POINTER)
}
pub const fn size(fields: &[Field]) -> usize {
    size_at(fields, POINTER)
}

/// Field `index` of a structure's bytes, as a 64-bit value: integers
/// zero-extended, pointers widened. `None` when `bytes` is too short.
pub fn get_at(bytes: &[u8], fields: &[Field], index: usize, width: usize) -> Option<u64> {
    get_packed_at(bytes, fields, index, width, 8)
}
/// `get_at` for a structure packed to `pack`.
pub fn get_packed_at(
    bytes: &[u8],
    fields: &[Field],
    index: usize,
    width: usize,
    pack: usize,
) -> Option<u64> {
    let at = offset_packed(fields, index, width, pack);
    let field = fields[index];
    let n = field.bytes(width);
    let raw = bytes.get(at..at + n)?;
    let mut word = [0u8; 8];
    Some(match field {
        Pointer if width == 4 => i64::from(i32::from_le_bytes(raw.try_into().ok()?)) as u64,
        U8 | U16 | U32 | U64 | Pointer => {
            word[..n].copy_from_slice(raw);
            u64::from_le_bytes(word)
        }
        Bytes(_) => return None,
    })
}
/// Store `value` as field `index` (truncated to the field's width).
pub fn set_at(bytes: &mut [u8], fields: &[Field], index: usize, width: usize, value: u64) -> bool {
    set_packed_at(bytes, fields, index, width, 8, value)
}
/// `set_at` for a structure packed to `pack`.
pub fn set_packed_at(
    bytes: &mut [u8],
    fields: &[Field],
    index: usize,
    width: usize,
    pack: usize,
    value: u64,
) -> bool {
    let at = offset_packed(fields, index, width, pack);
    let n = fields[index].bytes(width);
    if matches!(fields[index], Bytes(_)) {
        return false;
    }
    match bytes.get_mut(at..at + n) {
        Some(slot) => {
            slot.copy_from_slice(&value.to_le_bytes()[..n]);
            true
        }
        None => false,
    }
}
/// The bytes of an inline `Bytes` field (or any field) `index`.
pub fn field_bytes<'a>(
    bytes: &'a [u8],
    fields: &[Field],
    index: usize,
    width: usize,
) -> Option<&'a [u8]> {
    let at = offset_at(fields, index, width);
    bytes.get(at..at + fields[index].bytes(width))
}
/// `get_at` / `set_at` for this build.
pub fn get(bytes: &[u8], fields: &[Field], index: usize) -> u64 {
    get_at(bytes, fields, index, POINTER).unwrap_or(0)
}
pub fn set(bytes: &mut [u8], fields: &[Field], index: usize, value: u64) {
    set_at(bytes, fields, index, POINTER, value);
}
/// `get` / `set` / `offset` / `size` for this build at packing `pack`.
pub fn get_packed(bytes: &[u8], fields: &[Field], index: usize, pack: usize) -> u64 {
    get_packed_at(bytes, fields, index, POINTER, pack).unwrap_or(0)
}
pub fn set_packed(bytes: &mut [u8], fields: &[Field], index: usize, pack: usize, value: u64) {
    set_packed_at(bytes, fields, index, POINTER, pack, value);
}
pub const fn offset_pack(fields: &[Field], index: usize, pack: usize) -> usize {
    offset_packed(fields, index, POINTER, pack)
}
pub const fn size_pack(fields: &[Field], pack: usize) -> usize {
    size_packed(fields, POINTER, pack)
}

#[cfg(test)]
mod tests {
    use super::*;
    const WNDCLASSEXW: [Field; 12] = [
        U32, U32, Pointer, U32, U32, Pointer, Pointer, Pointer, Pointer, Pointer, Pointer, Pointer,
    ];
    // hwnd, message, wParam, lParam, time, pt (lPrivate is _MAC only).
    const MSG: [Field; 7] = [Pointer, U32, Pointer, Pointer, U32, U32, U32];
    const MEASUREITEMSTRUCT: [Field; 6] = [U32, U32, U32, U32, U32, Pointer];
    const WITH_U64: [Field; 2] = [U32, U64];
    #[test]
    fn layouts_match_msvc_for_both_widths() {
        assert_eq!(
            (size_at(&WNDCLASSEXW, 8), size_at(&WNDCLASSEXW, 4)),
            (80, 48)
        );
        assert_eq!(
            (offset_at(&WNDCLASSEXW, 2, 8), offset_at(&WNDCLASSEXW, 2, 4)),
            (8, 8)
        );
        assert_eq!(
            (offset_at(&WNDCLASSEXW, 5, 8), offset_at(&WNDCLASSEXW, 5, 4)),
            (24, 20)
        );
        assert_eq!(
            (
                offset_at(&WNDCLASSEXW, 10, 8),
                offset_at(&WNDCLASSEXW, 10, 4)
            ),
            (64, 40)
        );
        assert_eq!((size_at(&MSG, 8), size_at(&MSG, 4)), (48, 28));
        assert_eq!((offset_at(&MSG, 5, 8), offset_at(&MSG, 5, 4)), (36, 20));
        assert_eq!(
            (
                size_at(&MEASUREITEMSTRUCT, 8),
                size_at(&MEASUREITEMSTRUCT, 4)
            ),
            (32, 24)
        );
        // MSVC aligns an 8-byte integer to 8 in x86 structures too.
        assert_eq!((size_at(&WITH_U64, 4), offset_at(&WITH_U64, 1, 4)), (16, 8));
        assert_eq!(size_at(&[U16, Bytes(3)], 8), 6);
        // SHFILEOPSTRUCTW: x64 default packing 56; x86 pshpack1 30, with
        // fAnyOperationsAborted at the unaligned offset 18.
        const SHFILEOPSTRUCT: [Field; 8] =
            [Pointer, U32, Pointer, Pointer, U16, U32, Pointer, Pointer];
        assert_eq!(size_packed(&SHFILEOPSTRUCT, 8, 8), 56);
        assert_eq!(size_packed(&SHFILEOPSTRUCT, 4, 1), 30);
        assert_eq!(offset_packed(&SHFILEOPSTRUCT, 5, 4, 1), 18);
        assert_eq!(offset_packed(&SHFILEOPSTRUCT, 5, 8, 8), 36);
    }
    #[test]
    fn pointers_widen_and_store_at_the_width() {
        let mut b = [0u8; 28];
        assert!(set_at(&mut b, &MSG, 0, 4, (-3i64) as u64));
        assert!(set_at(&mut b, &MSG, 3, 4, 0x7fff_1234));
        assert_eq!(get_at(&b, &MSG, 0, 4), Some((-3i64) as u64));
        assert_eq!(get_at(&b, &MSG, 3, 4), Some(0x7fff_1234));
        assert_eq!(&b[0..4], &[0xfd, 0xff, 0xff, 0xff]);
        assert_eq!(get_at(&b[..20], &MSG, 6, 4), None);
        let mut w = [0u8; 48];
        set_at(&mut w, &MSG, 2, 8, 0x1_0000_0002);
        assert_eq!(get_at(&w, &MSG, 2, 8), Some(0x1_0000_0002));
    }
}
