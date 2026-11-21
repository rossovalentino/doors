//! Pre-publication negative checks against the authenticated, actual PE bundle.
//! Mutants exist only in a temporary owned NX mapping and never become runnable.
use super::*;
use door_image_format::PeImage;
use door_pe_loader::{ImageError, Reason};

const CASES: [(&str, Reason); 7] = [
    ("truncated", Reason::Image(ImageError::Truncated)),
    ("bad_machine", Reason::Image(ImageError::Unsupported)),
    ("missing_module", Reason::MissingModule),
    ("missing_symbol", Reason::MissingSymbol),
    ("collision", Reason::AddressCollision),
    ("missing_relocation", Reason::RelocationRequired),
    ("writable_code", Reason::Image(ImageError::Permissions)),
];

fn expect(sources: &[ModuleSource<'_>], case: usize) -> Result<(), Status> {
    match BundlePlan::new(sources) {
        Err(error) if error.reason == CASES[case].1 => Ok(()),
        Err(error) => {
            log(format_args!(
                "DOOR_P09 REJECT_FAIL case={} actual={:?}\n",
                CASES[case].0, error
            ));
            Err(Status::IoFailure)
        }
        Ok(_) => Err(Status::IoFailure),
    }
}
fn number(bytes: &[u8], offset: usize, width: usize) -> Result<u64, Status> {
    let source = bytes
        .get(offset..offset.checked_add(width).ok_or(Status::InvalidArgument)?)
        .ok_or(Status::InvalidArgument)?;
    let mut result = [0u8; 8];
    result
        .get_mut(..width)
        .ok_or(Status::InvalidArgument)?
        .copy_from_slice(source);
    Ok(u64::from_le_bytes(result))
}
fn parse(error: door_image_format::Error) -> Status {
    let _ = error;
    Status::InvalidArgument
}

pub(super) fn verify(original: &[ModuleSource<'_>; 3]) -> Result<(), Status> {
    // Validate the untouched bundle first, so every mutant has one intentional
    // defect and its expected refusal cannot hide an unrelated broken input.
    BundlePlan::new(original).map_err(failure)?;
    let source = original[0].bytes;
    let image = PeImage::parse(source).map_err(parse)?;
    let pe = usize::try_from(number(source, 0x3c, 4)?).map_err(|_| Status::InvalidArgument)?;
    let optional = pe.checked_add(24).ok_or(Status::InvalidArgument)?;
    let sections = optional
        .checked_add(number(source, pe + 20, 2)? as usize)
        .ok_or(Status::InvalidArgument)?;
    let (imports, _) = image.directory(1).map_err(parse)?;
    let descriptor = image.rva_bytes(imports, 20).map_err(parse)?;
    let original_thunk = number(descriptor, 0, 4)?;
    let thunk = if original_thunk == 0 {
        number(descriptor, 16, 4)?
    } else {
        original_thunk
    };
    let thunk = image
        .rva_bytes(
            u32::try_from(thunk).map_err(|_| Status::InvalidArgument)?,
            8,
        )
        .map_err(parse)?;
    let name = u32::try_from(number(thunk, 0, 8)?)
        .map_err(|_| Status::InvalidArgument)?
        .checked_add(2)
        .ok_or(Status::InvalidArgument)?;
    let symbol = image.file_offset(name, 1).map_err(parse)? as usize;
    let executable = (0..image.section_count())
        .find(|&i| image.section(i).is_ok_and(|s| s.flags & 0x2000_0000 != 0))
        .ok_or(Status::InvalidArgument)?;
    let flags = sections + executable * 40 + 36;
    let old_flags =
        u32::try_from(number(source, flags, 4)?).map_err(|_| Status::InvalidArgument)?;
    let baseline = with_meta(|m| m.executive.object_count());
    with_vm(|v| unsafe {
        v.with_windows_scratch(source.len(), |scratch| {
            let mut input = *original;
            input[0].bytes = source.get(..64).ok_or(Status::InvalidArgument)?;
            expect(&input, 0)?;
            scratch.copy_from_slice(source);
            scratch
                .get_mut(pe + 4..pe + 6)
                .ok_or(Status::InvalidArgument)?
                .copy_from_slice(&0x14cu16.to_le_bytes());
            input[0].bytes = scratch;
            expect(&input, 1)?;
            expect(&original[..2], 2)?;
            scratch.copy_from_slice(source);
            *scratch.get_mut(symbol).ok_or(Status::InvalidArgument)? = b'?';
            input = *original;
            input[0].bytes = scratch;
            expect(&input, 3)?;
            input = *original;
            input[1].base = input[0].base;
            expect(&input, 4)?;
            scratch.copy_from_slice(source);
            let relocation = optional + 112 + 5 * 8;
            scratch
                .get_mut(relocation..relocation + 8)
                .ok_or(Status::InvalidArgument)?
                .fill(0);
            input = *original;
            input[0].bytes = scratch;
            expect(&input, 5)?;
            scratch.copy_from_slice(source);
            scratch
                .get_mut(flags..flags + 4)
                .ok_or(Status::InvalidArgument)?
                .copy_from_slice(&(old_flags | 0x8000_0000).to_le_bytes());
            input = *original;
            input[0].bytes = scratch;
            expect(&input, 6)
        })
    })?;
    if with_meta(|m| m.executive.object_count()) != baseline {
        return Err(Status::IoFailure);
    }
    for (case, reason) in CASES {
        log(format_args!(
            "DOOR_P09 REJECT case={case} reason={reason:?} baseline=1\n"
        ));
    }
    Ok(())
}
