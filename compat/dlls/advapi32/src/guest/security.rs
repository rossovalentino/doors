//! Security descriptors, ACLs, SDDL, the access check, well-known SIDs,
//! account names, and the security of files (storage's ACLs), registry
//! keys and the process's own objects.
use super::*;
use crate::security::{
    self, Ace, Acl, Descriptor, Holder, RELATIVE_HEADER, ace, code, control, info,
};
use door_windows_abi::layout;

// ---- reading and writing descriptors --------------------------------------------------
fn read_acl(p: u64) -> Result<Acl, u32> {
    let mut head = [0u8; 8];
    read(p, &mut head)?;
    let size = usize::from(u16::from_le_bytes([head[2], head[3]]));
    if size < 8 {
        return Err(code::INVALID_ACL);
    }
    let mut b = vec![0u8; size];
    read(p, &mut b)?;
    Acl::decode(&b).ok_or(code::INVALID_ACL)
}
fn sid_length(p: u64) -> Result<usize, u32> {
    let mut head = [0u8; 8];
    read(p, &mut head)?;
    if head[0] != 1 || head[1] > 15 {
        return Err(error::INVALID_SID);
    }
    Ok(8 + 4 * usize::from(head[1]))
}
/// The absolute SECURITY_DESCRIPTOR at this build's width (P17): Revision,
/// Sbz1, Control, then the Owner, Group, Sacl and Dacl pointers (40 bytes
/// on x64, 20 on x86, SECURITY_DESCRIPTOR_MIN_LENGTH).
const ABSOLUTE: [layout::Field; 7] = {
    use layout::Field::*;
    [U8, U8, U16, Pointer, Pointer, Pointer, Pointer]
};
const ABSOLUTE_LENGTH: usize = layout::size(&ABSOLUTE);
const OWNER_SLOT: u64 = layout::offset(&ABSOLUTE, 3) as u64;
const GROUP_SLOT: u64 = layout::offset(&ABSOLUTE, 4) as u64;
const SACL_SLOT: u64 = layout::offset(&ABSOLUTE, 5) as u64;
const DACL_SLOT: u64 = layout::offset(&ABSOLUTE, 6) as u64;
fn control_of(p: u64) -> Result<u16, u32> {
    let mut head = [0u8; 4];
    read(p, &mut head)?;
    if head[0] != 1 {
        return Err(code::INVALID_SECURITY_DESCR);
    }
    Ok(u16::from_le_bytes([head[2], head[3]]))
}
/// The bytes of the self-relative descriptor at `p`.
fn relative_bytes(p: u64) -> Result<Vec<u8>, u32> {
    let mut head = [0u8; RELATIVE_HEADER];
    read(p, &mut head)?;
    let at = |slot: usize| u64::from(u32::from_le_bytes(head[slot..slot + 4].try_into().unwrap()));
    let mut end = RELATIVE_HEADER;
    for slot in [4, 8] {
        if at(slot) != 0 {
            end = end.max(at(slot) as usize + sid_length(p + at(slot))?);
        }
    }
    for slot in [12, 16] {
        if at(slot) != 0 {
            let mut acl = [0u8; 8];
            read(p + at(slot), &mut acl)?;
            end = end.max(at(slot) as usize + usize::from(u16::from_le_bytes([acl[2], acl[3]])));
        }
    }
    let mut b = vec![0u8; end];
    read(p, &mut b)?;
    Ok(b)
}
/// The descriptor at `p`, absolute or self-relative.
fn read_descriptor(p: u64) -> Result<Descriptor, u32> {
    if p == 0 {
        return Err(code::INVALID_SECURITY_DESCR);
    }
    let control = control_of(p)?;
    if control & control::SELF_RELATIVE != 0 {
        return Descriptor::from_relative(&relative_bytes(p)?).ok_or(code::INVALID_SECURITY_DESCR);
    }
    let owner = read_pointer(p + OWNER_SLOT)?;
    let group = read_pointer(p + GROUP_SLOT)?;
    let sacl = read_pointer(p + SACL_SLOT)?;
    let dacl = read_pointer(p + DACL_SLOT)?;
    Ok(Descriptor {
        control,
        owner: (owner != 0).then(|| read_sid(owner)).transpose()?,
        group: (group != 0).then(|| read_sid(group)).transpose()?,
        sacl: (sacl != 0 && control & control::SACL_PRESENT != 0)
            .then(|| read_acl(sacl))
            .transpose()?,
        dacl: (dacl != 0 && control & control::DACL_PRESENT != 0)
            .then(|| read_acl(dacl))
            .transpose()?,
    })
}
/// A LocalAlloc block holding `bytes` (LocalFree releases it).
fn local_copy(bytes: &[u8]) -> Result<u64, u32> {
    let p = unsafe {
        RtlAllocateHeap(
            door_windows_abi::narrow(DoorGetProcessHeap()),
            0,
            bytes.len().max(1),
        )
    };
    if p == 0 {
        return Err(error::NOT_ENOUGH_MEMORY);
    }
    write(door_windows_abi::widen(p), bytes)?;
    Ok(door_windows_abi::widen(p))
}
/// GetNamedSecurityInfo's outputs: the self-relative descriptor in a
/// LocalAlloc block, and the requested parts pointing into it.
fn hand_out(d: &Descriptor, outs: [u64; 5]) -> Result<(), u32> {
    let [owner, group, dacl, sacl, sd] = outs;
    if sd == 0 && (owner != 0 || group != 0 || dacl != 0 || sacl != 0) {
        return Err(error::INVALID_PARAMETER);
    }
    let bytes = d.relative();
    let at = |slot: usize| {
        u64::from(u32::from_le_bytes(
            bytes[slot..slot + 4].try_into().unwrap(),
        ))
    };
    let block = local_copy(&bytes)?;
    for (out, slot) in [(owner, 4), (group, 8), (sacl, 12), (dacl, 16)] {
        if out != 0 {
            let offset = at(slot);
            write_pointer(out, if offset == 0 { 0 } else { block + offset })?;
        }
    }
    if sd != 0 {
        write_pointer(sd, block)?;
    } else {
        unsafe {
            RtlFreeHeap(
                door_windows_abi::narrow(DoorGetProcessHeap()),
                0,
                door_windows_abi::narrow(block),
            )
        };
    }
    Ok(())
}
fn error_code(r: Result<(), u32>) -> u32 {
    let code = r.err().unwrap_or(error::SUCCESS);
    unsafe { RtlSetLastWin32Error(code) };
    code
}

// ---- absolute descriptors ---------------------------------------------------------------
#[unsafe(no_mangle)]
pub extern "system" fn InitializeSecurityDescriptor(p: usize, revision: u32) -> i32 {
    let p = door_windows_abi::widen(p);
    {
        let r = (|| {
            if revision != 1 {
                return Err(code::UNKNOWN_REVISION);
            }
            let mut b = [0u8; ABSOLUTE_LENGTH];
            b[0] = 1;
            write(p, &b)
        })();
        last_error(r.err().unwrap_or(error::SUCCESS))
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn IsValidSecurityDescriptor(p: usize) -> i32 {
    let p = door_windows_abi::widen(p);
    {
        last_error(
            read_descriptor(p)
                .map(|_| ())
                .err()
                .unwrap_or(error::SUCCESS),
        )
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn GetSecurityDescriptorLength(p: usize) -> u32 {
    let p = door_windows_abi::widen(p);
    {
        let Ok(control) = control_of(p) else {
            return 0;
        };
        if control & control::SELF_RELATIVE != 0 {
            return relative_bytes(p).map_or(0, |b| b.len() as u32);
        }
        // The absolute structure plus what its pointers reach.
        read_descriptor(p).map_or(0, |d| {
            (ABSOLUTE_LENGTH
                + d.owner.as_ref().map_or(0, Sid::length)
                + d.group.as_ref().map_or(0, Sid::length)
                + d.sacl.as_ref().map_or(0, Acl::length)
                + d.dacl.as_ref().map_or(0, Acl::length)) as u32
        })
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn GetSecurityDescriptorControl(p: usize, out: usize, revision: usize) -> i32 {
    let p = door_windows_abi::widen(p);
    let out = door_windows_abi::widen(out);
    let revision = door_windows_abi::widen(revision);
    {
        let r = (|| {
            let c = control_of(p)?;
            write(out, &c.to_le_bytes())?;
            write_u32(revision, 1)
        })();
        last_error(r.err().unwrap_or(error::SUCCESS))
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn SetSecurityDescriptorControl(p: usize, mask: u16, bits: u16) -> i32 {
    let p = door_windows_abi::widen(p);
    {
        let r = (|| {
            let c = control_of(p)?;
            if mask & !control::SETTABLE != 0 || bits & !mask != 0 {
                return Err(error::INVALID_PARAMETER);
            }
            write(p + 2, &((c & !mask) | bits).to_le_bytes())
        })();
        last_error(r.err().unwrap_or(error::SUCCESS))
    }
}
/// An absolute descriptor's slot (8 owner, 16 group, 24 SACL, 32 DACL).
fn absolute(p: u64) -> Result<u16, u32> {
    let c = control_of(p)?;
    if c & control::SELF_RELATIVE != 0 {
        return Err(code::INVALID_SECURITY_DESCR);
    }
    Ok(c)
}
/// Where a part is: the stored pointer, or the offset in a self-relative one.
fn part(p: u64, absolute_slot: u64, relative_slot: u64) -> Result<u64, u32> {
    let c = control_of(p)?;
    if c & control::SELF_RELATIVE != 0 {
        let offset = u64::from(read_u32(p + relative_slot)?);
        return Ok(if offset == 0 { 0 } else { p + offset });
    }
    read_pointer(p + absolute_slot)
}
fn set_part(p: u64, slot: u64, value: u64, flag: u16, on: bool) -> Result<(), u32> {
    let c = absolute(p)?;
    write_pointer(p + slot, value)?;
    let c = if on { c | flag } else { c & !flag };
    write(p + 2, &c.to_le_bytes())
}
#[unsafe(no_mangle)]
pub extern "system" fn GetSecurityDescriptorOwner(p: usize, out: usize, defaulted: usize) -> i32 {
    let p = door_windows_abi::widen(p);
    let out = door_windows_abi::widen(out);
    let defaulted = door_windows_abi::widen(defaulted);
    {
        let r = (|| {
            write_pointer(out, part(p, OWNER_SLOT, 4)?)?;
            write_u32(
                defaulted,
                u32::from(control_of(p)? & control::OWNER_DEFAULTED != 0),
            )
        })();
        last_error(r.err().unwrap_or(error::SUCCESS))
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn SetSecurityDescriptorOwner(p: usize, sid: usize, defaulted: i32) -> i32 {
    let p = door_windows_abi::widen(p);
    let sid = door_windows_abi::widen(sid);
    {
        last_error(
            set_part(p, OWNER_SLOT, sid, control::OWNER_DEFAULTED, defaulted != 0)
                .err()
                .unwrap_or(error::SUCCESS),
        )
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn GetSecurityDescriptorGroup(p: usize, out: usize, defaulted: usize) -> i32 {
    let p = door_windows_abi::widen(p);
    let out = door_windows_abi::widen(out);
    let defaulted = door_windows_abi::widen(defaulted);
    {
        let r = (|| {
            write_pointer(out, part(p, GROUP_SLOT, 8)?)?;
            write_u32(
                defaulted,
                u32::from(control_of(p)? & control::GROUP_DEFAULTED != 0),
            )
        })();
        last_error(r.err().unwrap_or(error::SUCCESS))
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn SetSecurityDescriptorGroup(p: usize, sid: usize, defaulted: i32) -> i32 {
    let p = door_windows_abi::widen(p);
    let sid = door_windows_abi::widen(sid);
    {
        last_error(
            set_part(p, GROUP_SLOT, sid, control::GROUP_DEFAULTED, defaulted != 0)
                .err()
                .unwrap_or(error::SUCCESS),
        )
    }
}
fn get_acl(p: u64, which: (u64, u64, u16, u16), present: u64, out: u64, defaulted: u64) -> i32 {
    let (absolute_slot, relative_slot, present_bit, defaulted_bit) = which;
    let r = (|| {
        let c = control_of(p)?;
        let on = c & present_bit != 0;
        write_u32(present, u32::from(on))?;
        if on {
            write_pointer(out, part(p, absolute_slot, relative_slot)?)?;
            write_u32(defaulted, u32::from(c & defaulted_bit != 0))?;
        }
        Ok(())
    })();
    last_error(r.err().unwrap_or(error::SUCCESS))
}
fn set_acl(p: u64, slot: u64, bits: (u16, u16), present: i32, acl: u64, defaulted: i32) -> i32 {
    let r = (|| {
        let c = absolute(p)?;
        let (present_bit, defaulted_bit) = bits;
        let mut c = c & !(present_bit | defaulted_bit);
        if present != 0 {
            c |= present_bit;
            if defaulted != 0 {
                c |= defaulted_bit;
            }
        }
        write_pointer(p + slot, if present != 0 { acl } else { 0 })?;
        write(p + 2, &c.to_le_bytes())
    })();
    last_error(r.err().unwrap_or(error::SUCCESS))
}
#[unsafe(no_mangle)]
pub extern "system" fn GetSecurityDescriptorDacl(
    p: usize,
    present: usize,
    out: usize,
    defaulted: usize,
) -> i32 {
    let p = door_windows_abi::widen(p);
    let present = door_windows_abi::widen(present);
    let out = door_windows_abi::widen(out);
    let defaulted = door_windows_abi::widen(defaulted);
    {
        get_acl(
            p,
            (
                DACL_SLOT,
                16,
                control::DACL_PRESENT,
                control::DACL_DEFAULTED,
            ),
            present,
            out,
            defaulted,
        )
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn SetSecurityDescriptorDacl(
    p: usize,
    present: i32,
    acl: usize,
    defaulted: i32,
) -> i32 {
    let p = door_windows_abi::widen(p);
    let acl = door_windows_abi::widen(acl);
    {
        set_acl(
            p,
            DACL_SLOT,
            (control::DACL_PRESENT, control::DACL_DEFAULTED),
            present,
            acl,
            defaulted,
        )
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn GetSecurityDescriptorSacl(
    p: usize,
    present: usize,
    out: usize,
    defaulted: usize,
) -> i32 {
    let p = door_windows_abi::widen(p);
    let present = door_windows_abi::widen(present);
    let out = door_windows_abi::widen(out);
    let defaulted = door_windows_abi::widen(defaulted);
    {
        get_acl(
            p,
            (
                SACL_SLOT,
                12,
                control::SACL_PRESENT,
                control::SACL_DEFAULTED,
            ),
            present,
            out,
            defaulted,
        )
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn SetSecurityDescriptorSacl(
    p: usize,
    present: i32,
    acl: usize,
    defaulted: i32,
) -> i32 {
    let p = door_windows_abi::widen(p);
    let acl = door_windows_abi::widen(acl);
    {
        set_acl(
            p,
            SACL_SLOT,
            (control::SACL_PRESENT, control::SACL_DEFAULTED),
            present,
            acl,
            defaulted,
        )
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn MakeSelfRelativeSD(p: usize, out: usize, length: usize) -> i32 {
    let p = door_windows_abi::widen(p);
    let out = door_windows_abi::widen(out);
    let length = door_windows_abi::widen(length);
    {
        let r = (|| {
            if control_of(p)? & control::SELF_RELATIVE != 0 {
                return Err(code::BAD_DESCRIPTOR_FORMAT);
            }
            let bytes = read_descriptor(p)?.relative();
            let have = read_u32(length)?;
            if (have as usize) < bytes.len() {
                write_u32(length, bytes.len() as u32)?;
                return Err(error::INSUFFICIENT_BUFFER);
            }
            write(out, &bytes)
        })();
        last_error(r.err().unwrap_or(error::SUCCESS))
    }
}
#[unsafe(no_mangle)]
#[allow(clippy::too_many_arguments)]
pub extern "system" fn MakeAbsoluteSD(
    p: usize,
    out: usize,
    out_length: usize,
    dacl: usize,
    dacl_length: usize,
    sacl: usize,
    sacl_length: usize,
    owner: usize,
    owner_length: usize,
    group: usize,
    group_length: usize,
) -> i32 {
    let p = door_windows_abi::widen(p);
    let out = door_windows_abi::widen(out);
    let out_length = door_windows_abi::widen(out_length);
    let dacl = door_windows_abi::widen(dacl);
    let dacl_length = door_windows_abi::widen(dacl_length);
    let sacl = door_windows_abi::widen(sacl);
    let sacl_length = door_windows_abi::widen(sacl_length);
    let owner = door_windows_abi::widen(owner);
    let owner_length = door_windows_abi::widen(owner_length);
    let group = door_windows_abi::widen(group);
    let group_length = door_windows_abi::widen(group_length);
    {
        let r = (|| {
            if control_of(p)? & control::SELF_RELATIVE == 0 {
                return Err(code::BAD_DESCRIPTOR_FORMAT);
            }
            let d = read_descriptor(p)?;
            let parts = [
                (out_length, ABSOLUTE_LENGTH),
                (dacl_length, d.dacl.as_ref().map_or(0, Acl::length)),
                (sacl_length, d.sacl.as_ref().map_or(0, Acl::length)),
                (owner_length, d.owner.as_ref().map_or(0, Sid::length)),
                (group_length, d.group.as_ref().map_or(0, Sid::length)),
            ];
            let mut short = false;
            for (length, need) in parts {
                if (read_u32(length)? as usize) < need {
                    short = true;
                }
            }
            if short {
                for (length, need) in parts {
                    write_u32(length, need as u32)?;
                }
                return Err(error::INSUFFICIENT_BUFFER);
            }
            let mut b = [0u8; ABSOLUTE_LENGTH];
            b[0] = 1;
            b[2..4].copy_from_slice(&d.control.to_le_bytes());
            let mut place =
                |index: usize, target: u64, bytes: Option<Vec<u8>>| -> Result<(), u32> {
                    if let Some(bytes) = bytes {
                        write(target, &bytes)?;
                        layout::set(&mut b, &ABSOLUTE, index, target);
                    }
                    Ok(())
                };
            place(3, owner, d.owner.as_ref().map(Sid::encode))?;
            place(4, group, d.group.as_ref().map(Sid::encode))?;
            place(5, sacl, d.sacl.as_ref().map(Acl::encode))?;
            place(6, dacl, d.dacl.as_ref().map(Acl::encode))?;
            for (length, need) in parts {
                write_u32(length, need as u32)?;
            }
            write(out, &b)
        })();
        last_error(r.err().unwrap_or(error::SUCCESS))
    }
}

// ---- ACLs in the caller's memory ----------------------------------------------------------
/// (AclSize, AceCount, bytes in use) of the ACL at `p`.
fn acl_usage(p: u64) -> Result<(usize, usize, usize), u32> {
    let mut head = [0u8; 8];
    read(p, &mut head)?;
    let mut b = vec![0u8; usize::from(u16::from_le_bytes([head[2], head[3]])).max(8)];
    read(p, &mut b)?;
    let (size, count) = security::header(&b).ok_or(code::INVALID_ACL)?;
    let mut used = 8;
    for _ in 0..count {
        let (_, n) = Ace::decode(&b[used..size]).ok_or(code::INVALID_ACL)?;
        used += n;
    }
    Ok((size, count, used))
}
#[unsafe(no_mangle)]
pub extern "system" fn InitializeAcl(p: usize, length: u32, revision: u32) -> i32 {
    let p = door_windows_abi::widen(p);
    {
        let r = (|| {
            if length < 8 {
                return Err(error::INSUFFICIENT_BUFFER);
            }
            if length > 0xFFFF || !(2..=4).contains(&revision) {
                return Err(error::INVALID_PARAMETER);
            }
            let mut b = [0u8; 8];
            b[0] = revision as u8;
            b[2..4].copy_from_slice(&((length & !3) as u16).to_le_bytes());
            write(p, &b)
        })();
        last_error(r.err().unwrap_or(error::SUCCESS))
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn IsValidAcl(p: usize) -> i32 {
    let p = door_windows_abi::widen(p);
    i32::from(acl_usage(p).is_ok())
}
/// Insert raw ACEs at `index` (u32::MAX: the end).
fn insert(p: u64, index: u32, aces: &[u8], count: usize) -> Result<(), u32> {
    let (size, existing, used) = acl_usage(p)?;
    if used + aces.len() > size {
        return Err(code::ALLOTTED_SPACE_EXCEEDED);
    }
    let mut b = vec![0u8; used];
    read(p, &mut b)?;
    // The byte offset of ACE `index`.
    let mut at = 8;
    let stop = (index as usize).min(existing);
    for _ in 0..stop {
        let (_, n) = Ace::decode(&b[at..]).ok_or(code::INVALID_ACL)?;
        at += n;
    }
    let mut out = b[..at].to_vec();
    out.extend_from_slice(aces);
    out.extend_from_slice(&b[at..]);
    out[4..6].copy_from_slice(&((existing + count) as u16).to_le_bytes());
    write(p, &out)
}
fn add(p: u64, kind: u8, flags: u32, mask: u32, sid: u64) -> i32 {
    let r = (|| {
        if flags & !0xDF != 0 {
            return Err(code::INVALID_FLAGS);
        }
        let sid = read_sid(sid)?;
        insert(
            p,
            u32::MAX,
            &Ace::new(kind, flags as u8, mask, sid).encode(),
            1,
        )
    })();
    last_error(r.err().unwrap_or(error::SUCCESS))
}
#[unsafe(no_mangle)]
pub extern "system" fn AddAccessAllowedAce(p: usize, _revision: u32, mask: u32, sid: usize) -> i32 {
    let p = door_windows_abi::widen(p);
    let sid = door_windows_abi::widen(sid);
    add(p, ace::ALLOWED, 0, mask, sid)
}
#[unsafe(no_mangle)]
pub extern "system" fn AddAccessAllowedAceEx(
    p: usize,
    _revision: u32,
    flags: u32,
    mask: u32,
    sid: usize,
) -> i32 {
    let p = door_windows_abi::widen(p);
    let sid = door_windows_abi::widen(sid);
    add(p, ace::ALLOWED, flags, mask, sid)
}
#[unsafe(no_mangle)]
pub extern "system" fn AddAccessDeniedAce(p: usize, _revision: u32, mask: u32, sid: usize) -> i32 {
    let p = door_windows_abi::widen(p);
    let sid = door_windows_abi::widen(sid);
    add(p, ace::DENIED, 0, mask, sid)
}
#[unsafe(no_mangle)]
pub extern "system" fn AddAccessDeniedAceEx(
    p: usize,
    _revision: u32,
    flags: u32,
    mask: u32,
    sid: usize,
) -> i32 {
    let p = door_windows_abi::widen(p);
    let sid = door_windows_abi::widen(sid);
    add(p, ace::DENIED, flags, mask, sid)
}
#[unsafe(no_mangle)]
pub extern "system" fn AddMandatoryAce(
    p: usize,
    _revision: u32,
    flags: u32,
    policy: u32,
    sid: usize,
) -> i32 {
    let p = door_windows_abi::widen(p);
    let sid = door_windows_abi::widen(sid);
    add(p, ace::LABEL, flags, policy, sid)
}
#[unsafe(no_mangle)]
pub extern "system" fn AddAce(
    p: usize,
    _revision: u32,
    index: u32,
    list: usize,
    length: u32,
) -> i32 {
    let p = door_windows_abi::widen(p);
    let list = door_windows_abi::widen(list);
    {
        let r = (|| {
            let mut b = vec![0u8; length as usize];
            read(list, &mut b)?;
            let mut at = 0;
            let mut count = 0;
            while at < b.len() {
                let (_, n) = Ace::decode(&b[at..]).ok_or(error::INVALID_PARAMETER)?;
                at += n;
                count += 1;
            }
            insert(p, index, &b, count)
        })();
        last_error(r.err().unwrap_or(error::SUCCESS))
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn GetAce(p: usize, index: u32, out: usize) -> i32 {
    let p = door_windows_abi::widen(p);
    let out = door_windows_abi::widen(out);
    {
        let r = (|| {
            let (_, count, used) = acl_usage(p)?;
            if index as usize >= count {
                return Err(error::INVALID_PARAMETER);
            }
            let mut b = vec![0u8; used];
            read(p, &mut b)?;
            let mut at = 8;
            for _ in 0..index {
                at += Ace::decode(&b[at..]).ok_or(code::INVALID_ACL)?.1;
            }
            write_pointer(out, p + at as u64)
        })();
        last_error(r.err().unwrap_or(error::SUCCESS))
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn DeleteAce(p: usize, index: u32) -> i32 {
    let p = door_windows_abi::widen(p);
    {
        let r = (|| {
            let (_, count, used) = acl_usage(p)?;
            if index as usize >= count {
                return Err(error::INVALID_PARAMETER);
            }
            let mut b = vec![0u8; used];
            read(p, &mut b)?;
            let mut at = 8;
            for _ in 0..index {
                at += Ace::decode(&b[at..]).ok_or(code::INVALID_ACL)?.1;
            }
            let n = Ace::decode(&b[at..]).ok_or(code::INVALID_ACL)?.1;
            let mut out = b[..at].to_vec();
            out.extend_from_slice(&b[at + n..]);
            out[4..6].copy_from_slice(&((count - 1) as u16).to_le_bytes());
            // Zero the freed tail.
            out.resize(used, 0);
            write(p, &out)
        })();
        last_error(r.err().unwrap_or(error::SUCCESS))
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn GetAclInformation(p: usize, out: usize, length: u32, class: u32) -> i32 {
    let p = door_windows_abi::widen(p);
    let out = door_windows_abi::widen(out);
    {
        let r = (|| {
            let (size, count, used) = acl_usage(p)?;
            match class {
                // AclRevisionInformation.
                1 => {
                    if length < 4 {
                        return Err(error::INSUFFICIENT_BUFFER);
                    }
                    let mut head = [0u8; 1];
                    read(p, &mut head)?;
                    write_u32(out, u32::from(head[0]))
                }
                // AclSizeInformation: count, in use, free.
                2 => {
                    if length < 12 {
                        return Err(error::INSUFFICIENT_BUFFER);
                    }
                    write_u32(out, count as u32)?;
                    write_u32(out + 4, used as u32)?;
                    write_u32(out + 8, (size - used) as u32)
                }
                _ => Err(error::INVALID_PARAMETER),
            }
        })();
        last_error(r.err().unwrap_or(error::SUCCESS))
    }
}

// ---- SDDL ---------------------------------------------------------------------------------
fn sddl_to_descriptor(text_ptr: u64, revision: u32, out: u64, size: u64, wide: bool) -> i32 {
    let r = (|| {
        if revision != 1 {
            return Err(code::UNKNOWN_REVISION);
        }
        let s = String::from_utf16_lossy(&text(text_ptr, wide)?);
        let bytes = security::from_sddl(&s)?.relative();
        let block = local_copy(&bytes)?;
        if size != 0 {
            write_u32(size, bytes.len() as u32)?;
        }
        write_pointer(out, block)
    })();
    last_error(r.err().unwrap_or(error::SUCCESS))
}
#[unsafe(no_mangle)]
pub extern "system" fn ConvertStringSecurityDescriptorToSecurityDescriptorW(
    text_ptr: usize,
    revision: u32,
    out: usize,
    size: usize,
) -> i32 {
    let text_ptr = door_windows_abi::widen(text_ptr);
    let out = door_windows_abi::widen(out);
    let size = door_windows_abi::widen(size);
    sddl_to_descriptor(text_ptr, revision, out, size, true)
}
#[unsafe(no_mangle)]
pub extern "system" fn ConvertStringSecurityDescriptorToSecurityDescriptorA(
    text_ptr: usize,
    revision: u32,
    out: usize,
    size: usize,
) -> i32 {
    let text_ptr = door_windows_abi::widen(text_ptr);
    let out = door_windows_abi::widen(out);
    let size = door_windows_abi::widen(size);
    sddl_to_descriptor(text_ptr, revision, out, size, false)
}
fn descriptor_to_sddl(p: u64, revision: u32, which: u32, out: u64, length: u64, wide: bool) -> i32 {
    let r = (|| {
        if revision != 1 {
            return Err(code::UNKNOWN_REVISION);
        }
        let d = read_descriptor(p)?;
        let s = security::to_sddl(&d, which).ok_or(error::INVALID_PARAMETER)?;
        let units: Vec<u16> = s.encode_utf16().collect();
        let bytes = (units.len() + 1) * if wide { 2 } else { 1 };
        let block =
            unsafe { RtlAllocateHeap(door_windows_abi::narrow(DoorGetProcessHeap()), 0, bytes) };
        if block == 0 {
            return Err(error::NOT_ENOUGH_MEMORY);
        }
        put_text(
            door_windows_abi::widen(block),
            units.len() as u32 + 1,
            &units,
            wide,
        )?;
        if length != 0 {
            write_u32(length, security::sddl_length(&d, which, &s) as u32)?;
        }
        write_pointer(out, door_windows_abi::widen(block))
    })();
    last_error(r.err().unwrap_or(error::SUCCESS))
}
#[unsafe(no_mangle)]
pub extern "system" fn ConvertSecurityDescriptorToStringSecurityDescriptorW(
    p: usize,
    revision: u32,
    which: u32,
    out: usize,
    length: usize,
) -> i32 {
    let p = door_windows_abi::widen(p);
    let out = door_windows_abi::widen(out);
    let length = door_windows_abi::widen(length);
    descriptor_to_sddl(p, revision, which, out, length, true)
}
#[unsafe(no_mangle)]
pub extern "system" fn ConvertSecurityDescriptorToStringSecurityDescriptorA(
    p: usize,
    revision: u32,
    which: u32,
    out: usize,
    length: usize,
) -> i32 {
    let p = door_windows_abi::widen(p);
    let out = door_windows_abi::widen(out);
    let length = door_windows_abi::widen(length);
    descriptor_to_sddl(p, revision, which, out, length, false)
}

// ---- access checks --------------------------------------------------------------------------
fn read_mapping(p: u64) -> Result<security::Mapping, u32> {
    Ok([
        read_u32(p)?,
        read_u32(p + 4)?,
        read_u32(p + 8)?,
        read_u32(p + 12)?,
    ])
}
#[unsafe(no_mangle)]
pub extern "system" fn MapGenericMask(mask: usize, mapping: usize) {
    let mask = door_windows_abi::widen(mask);
    let mapping = door_windows_abi::widen(mapping);
    {
        if let (Ok(m), Ok(map)) = (read_u32(mask), read_mapping(mapping)) {
            let _ = write_u32(mask, security::map_generic(m, &map));
        }
    }
}
const GENERIC_BITS: u32 = 0xF000_0000;
#[unsafe(no_mangle)]
#[allow(clippy::too_many_arguments)]
pub extern "system" fn AccessCheck(
    sd: usize,
    token: usize,
    desired: u32,
    mapping: usize,
    privileges: usize,
    privileges_length: usize,
    granted: usize,
    status: usize,
) -> i32 {
    let sd = door_windows_abi::widen(sd);
    let token = door_windows_abi::widen(token);
    let mapping = door_windows_abi::widen(mapping);
    let privileges = door_windows_abi::widen(privileges);
    let privileges_length = door_windows_abi::widen(privileges_length);
    let granted = door_windows_abi::widen(granted);
    let status = door_windows_abi::widen(status);
    {
        let r = (|| {
            // Generic rights must be mapped first (MapGenericMask).
            if desired & GENERIC_BITS != 0 {
                return Err(code::GENERIC_NOT_MAPPED);
            }
            if token == 0 {
                return Err(error::INVALID_HANDLE);
            }
            let token = checking_token(token)?;
            // PRIVILEGE_SET with room for one privilege.
            if read_u32(privileges_length)? < 20 {
                write_u32(privileges_length, 20)?;
                return Err(error::INSUFFICIENT_BUFFER);
            }
            let d = read_descriptor(sd)?;
            let map = read_mapping(mapping)?;
            let user = Sid::user(token.uid);
            let groups = token.groups();
            let holder = Holder {
                user: &user,
                groups: &groups,
            };
            write_u32(privileges, 0)?;
            write_u32(privileges + 4, 0)?;
            write_u32(privileges_length, 20)?;
            match security::access_check(&d, &holder, desired, &map) {
                Ok(mask) => {
                    write_u32(granted, mask)?;
                    write_u32(status, 1)?;
                    Ok(error::SUCCESS)
                }
                Err(error::ACCESS_DENIED) => {
                    write_u32(granted, 0)?;
                    write_u32(status, 0)?;
                    Ok(error::ACCESS_DENIED)
                }
                Err(e) => Err(e),
            }
        })();
        match r {
            // The call succeeded; a refusal is in AccessStatus and the last error.
            Ok(code) => {
                unsafe { RtlSetLastWin32Error(code) };
                1
            }
            Err(e) => last_error(e),
        }
    }
}

// ---- SIDs and names ------------------------------------------------------------------------
/// The domain-relative well-known SIDs (WinAccount*): type and RID.
const DOMAIN_RIDS: [(u32, u32); 13] = [
    (38, 500),
    (39, 501),
    (40, 502),
    (41, 512),
    (42, 513),
    (43, 514),
    (44, 515),
    (45, 516),
    (46, 517),
    (47, 518),
    (48, 519),
    (49, 520),
    (50, 553),
];
#[unsafe(no_mangle)]
pub extern "system" fn CreateWellKnownSid(
    kind: u32,
    domain: usize,
    out: usize,
    size: usize,
) -> i32 {
    let domain = door_windows_abi::widen(domain);
    let out = door_windows_abi::widen(out);
    let size = door_windows_abi::widen(size);
    {
        let r = (|| {
            let sid = match security::well_known(kind) {
                Some(sid) => sid,
                None => {
                    let rid = DOMAIN_RIDS
                        .iter()
                        .find(|(k, _)| *k == kind)
                        .map(|(_, r)| *r)
                        .ok_or(error::INVALID_PARAMETER)?;
                    if domain == 0 {
                        return Err(error::INVALID_PARAMETER);
                    }
                    let mut d = read_sid(domain)?;
                    d.sub.push(rid);
                    d
                }
            };
            let have = read_u32(size)?;
            write_u32(size, sid.length() as u32)?;
            if (have as usize) < sid.length() {
                return Err(error::INSUFFICIENT_BUFFER);
            }
            write(out, &sid.encode())
        })();
        last_error(r.err().unwrap_or(error::SUCCESS))
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn IsWellKnownSid(sid: usize, kind: u32) -> i32 {
    let sid = door_windows_abi::widen(sid);
    i32::from(read_sid(sid).is_ok_and(|s| security::is_well_known(&s, kind)))
}
#[unsafe(no_mangle)]
pub extern "system" fn GetWindowsAccountDomainSid(sid: usize, out: usize, size: usize) -> i32 {
    let sid = door_windows_abi::widen(sid);
    let out = door_windows_abi::widen(out);
    let size = door_windows_abi::widen(size);
    {
        let r = (|| {
            let sid = read_sid(sid)?;
            let domain = match (sid.authority, sid.sub.as_slice()) {
                (5, [21, a, b, c, ..]) => Sid::new(5, &[21, *a, *b, *c]),
                _ => return Err(error::NONE_MAPPED),
            };
            let have = read_u32(size)?;
            write_u32(size, domain.length() as u32)?;
            if (have as usize) < domain.length() {
                return Err(error::INSUFFICIENT_BUFFER);
            }
            write(out, &domain.encode())
        })();
        last_error(r.err().unwrap_or(error::SUCCESS))
    }
}
#[allow(clippy::too_many_arguments)]
fn lookup_account_name(
    name: u64,
    sid: u64,
    sid_size: u64,
    domain: u64,
    domain_size: u64,
    use_out: u64,
    wide: bool,
) -> i32 {
    let r = (|| {
        let wanted = String::from_utf16_lossy(&text(name, wide)?);
        let computer = String::from_utf16_lossy(&computer_name());
        let (uid, me) = me();
        let (found, dom, kind) =
            security::lookup_name(&wanted, &computer, (uid, &me)).ok_or(error::NONE_MAPPED)?;
        let dom: Vec<u16> = dom.encode_utf16().collect();
        let (have_sid, have_domain) = (read_u32(sid_size)?, read_u32(domain_size)?);
        if (have_sid as usize) < found.length() || (have_domain as usize) < dom.len() + 1 {
            write_u32(sid_size, found.length() as u32)?;
            write_u32(domain_size, dom.len() as u32 + 1)?;
            return Err(error::INSUFFICIENT_BUFFER);
        }
        write(sid, &found.encode())?;
        write_u32(sid_size, found.length() as u32)?;
        put_text(domain, have_domain, &dom, wide)?;
        write_u32(domain_size, dom.len() as u32)?;
        write_u32(use_out, kind)
    })();
    last_error(r.err().unwrap_or(error::SUCCESS))
}
#[unsafe(no_mangle)]
#[allow(clippy::too_many_arguments)]
pub extern "system" fn LookupAccountNameW(
    _system: usize,
    name: usize,
    sid: usize,
    sid_size: usize,
    domain: usize,
    domain_size: usize,
    use_out: usize,
) -> i32 {
    let name = door_windows_abi::widen(name);
    let sid = door_windows_abi::widen(sid);
    let sid_size = door_windows_abi::widen(sid_size);
    let domain = door_windows_abi::widen(domain);
    let domain_size = door_windows_abi::widen(domain_size);
    let use_out = door_windows_abi::widen(use_out);
    lookup_account_name(name, sid, sid_size, domain, domain_size, use_out, true)
}
#[unsafe(no_mangle)]
#[allow(clippy::too_many_arguments)]
pub extern "system" fn LookupAccountNameA(
    _system: usize,
    name: usize,
    sid: usize,
    sid_size: usize,
    domain: usize,
    domain_size: usize,
    use_out: usize,
) -> i32 {
    let name = door_windows_abi::widen(name);
    let sid = door_windows_abi::widen(sid);
    let sid_size = door_windows_abi::widen(sid_size);
    let domain = door_windows_abi::widen(domain);
    let domain_size = door_windows_abi::widen(domain_size);
    let use_out = door_windows_abi::widen(use_out);
    lookup_account_name(name, sid, sid_size, domain, domain_size, use_out, false)
}

// ---- object security --------------------------------------------------------------------------
/// SE_OBJECT_TYPE.
const SE_FILE_OBJECT: u32 = 1;
const SE_REGISTRY_KEY: u32 = 4;
const SE_KERNEL_OBJECT: u32 = 6;
/// A file name as storage's path (relative names resolve against the
/// current directory), and whether its parent exists (for the error).
fn storage_path(name: &[u16]) -> Result<String, u32> {
    let mut w: Vec<u16> = name.to_vec();
    w.push(0);
    let mut out = vec![0u8; door_service_protocol::CHUNK_BYTES];
    let n =
        unsafe { DoorGetNativePath(w.as_ptr() as u64, out.as_mut_ptr() as u64, out.len() as u32) }
            as usize;
    if n == 0 || n > out.len() {
        return Err(unsafe { RtlGetLastWin32Error() }.max(1));
    }
    String::from_utf8(out[..n].to_vec()).map_err(|_| error::INVALID_PARAMETER)
}
/// A missing file: ERROR_FILE_NOT_FOUND when its folder exists, else
/// ERROR_PATH_NOT_FOUND.
fn missing(name: &[u16]) -> u32 {
    let Some(cut) = name
        .iter()
        .rposition(|u| *u == u16::from(b'\\') || *u == u16::from(b'/'))
    else {
        return error::FILE_NOT_FOUND;
    };
    let mut parent = name[..cut.max(3)].to_vec();
    parent.push(0);
    if unsafe { DoorGetFileAttributesW(parent.as_ptr() as u64) } == u32::MAX {
        3
    } else {
        error::FILE_NOT_FOUND
    }
}
fn file_security(name: &[u16]) -> Result<door_service_protocol::files::Security, u32> {
    let path = storage_path(name)?;
    storage(|s| {
        s.security(&path).map_err(|e| match win32(e) {
            error::FILE_NOT_FOUND => missing(name),
            other => other,
        })
    })
}
/// A registry object name ("MACHINE\SOFTWARE\...", "CURRENT_USER\...",
/// "CLASSES_ROOT\...", "USERS\...") as an existing key's descriptor.
fn key_security(name: &[u16]) -> Result<Descriptor, u32> {
    let text = String::from_utf16_lossy(name);
    let (root, rest) = text.split_once('\\').unwrap_or((text.as_str(), ""));
    let key = match root.to_ascii_uppercase().as_str() {
        "MACHINE" => hkey::LOCAL_MACHINE,
        "CURRENT_USER" => hkey::CURRENT_USER,
        "CLASSES_ROOT" => hkey::CLASSES_ROOT,
        "USERS" => hkey::USERS,
        _ => return Err(error::INVALID_PARAMETER),
    };
    let rest: Vec<u16> = rest.encode_utf16().collect();
    let open = resolve(key, &rest, sam::QUERY_VALUE)?;
    with_key(&open, false, |_, _| Ok(()))?;
    Ok(security::key_descriptor(
        open.root != RegistryRoot::CurrentUser,
        identity().uid,
    ))
}
/// The process's or a thread's own object: owned by the token's owner, its
/// DACL the token's default one with the object's full access (and
/// Authenticated Users' limited query), as Windows reports a process.
fn kernel_security(handle: u64) -> Result<Descriptor, u32> {
    let (all, query) = match handle {
        u64::MAX => (0x1F_FFFF, 0x1000),
        0xFFFF_FFFF_FFFF_FFFE => (0x1F_FFFF, 0x800),
        _ => return Err(error::NOT_SUPPORTED),
    };
    let token = identity();
    let mut aces = Vec::new();
    for (i, a) in token.default_dacl().aces.into_iter().enumerate() {
        aces.push(Ace::new(ace::ALLOWED, 0, all, a.sid));
        if i == 0 {
            aces.push(Ace::new(ace::ALLOWED, 0, query, Sid::authenticated()));
        }
    }
    Ok(Descriptor {
        control: control::DACL_PRESENT,
        owner: Some(token.owner()),
        group: Some(Sid::user(token.uid)),
        dacl: Some(Acl { aces }),
        sacl: None,
    })
}
/// The parts `which` selects. Reading a SACL needs SeSecurityPrivilege
/// enabled, which Doors' tokens never have enabled.
fn selected(d: &Descriptor, which: u32) -> Result<Descriptor, u32> {
    if which & info::SACL != 0 {
        return Err(code::PRIVILEGE_NOT_HELD);
    }
    Ok(d.select(which))
}
fn named_descriptor(name: &[u16], kind: u32) -> Result<Descriptor, u32> {
    match kind {
        SE_FILE_OBJECT => Ok(security::descriptor_of(&file_security(name)?)),
        SE_REGISTRY_KEY => key_security(name),
        _ => Err(error::INVALID_PARAMETER),
    }
}
#[allow(clippy::too_many_arguments)]
fn get_named(
    name: u64,
    kind: u32,
    which: u32,
    owner: u64,
    group: u64,
    dacl: u64,
    sacl: u64,
    sd: u64,
    wide: bool,
) -> u32 {
    error_code((|| {
        let d = named_descriptor(&text(name, wide)?, kind)?;
        hand_out(&selected(&d, which)?, [owner, group, dacl, sacl, sd])
    })())
}
#[unsafe(no_mangle)]
#[allow(clippy::too_many_arguments)]
pub extern "system" fn GetNamedSecurityInfoW(
    name: usize,
    kind: u32,
    which: u32,
    owner: usize,
    group: usize,
    dacl: usize,
    sacl: usize,
    sd: usize,
) -> u32 {
    let name = door_windows_abi::widen(name);
    let owner = door_windows_abi::widen(owner);
    let group = door_windows_abi::widen(group);
    let dacl = door_windows_abi::widen(dacl);
    let sacl = door_windows_abi::widen(sacl);
    let sd = door_windows_abi::widen(sd);
    get_named(name, kind, which, owner, group, dacl, sacl, sd, true)
}
#[unsafe(no_mangle)]
#[allow(clippy::too_many_arguments)]
pub extern "system" fn GetNamedSecurityInfoA(
    name: usize,
    kind: u32,
    which: u32,
    owner: usize,
    group: usize,
    dacl: usize,
    sacl: usize,
    sd: usize,
) -> u32 {
    let name = door_windows_abi::widen(name);
    let owner = door_windows_abi::widen(owner);
    let group = door_windows_abi::widen(group);
    let dacl = door_windows_abi::widen(dacl);
    let sacl = door_windows_abi::widen(sacl);
    let sd = door_windows_abi::widen(sd);
    get_named(name, kind, which, owner, group, dacl, sacl, sd, false)
}
/// The registry key behind an open HKEY.
fn open_key_security(key: u64) -> Result<Descriptor, u32> {
    let open = resolve(key, &[], sam::QUERY_VALUE)?;
    Ok(security::key_descriptor(
        open.root != RegistryRoot::CurrentUser,
        identity().uid,
    ))
}
fn file_handle_security(handle: u64) -> Result<door_service_protocol::files::Security, u32> {
    use door_service_protocol::files::{SECURITY_BYTES, Security};
    let mut b = [0u8; SECURITY_BYTES];
    if unsafe { DoorGetFileSecurity(handle, b.as_mut_ptr() as u64) } == 0 {
        return Err(unsafe { RtlGetLastWin32Error() }.max(1));
    }
    Security::decode(&b).map_err(|_| error::INVALID_DATA)
}
#[unsafe(no_mangle)]
#[allow(clippy::too_many_arguments)]
pub extern "system" fn GetSecurityInfo(
    handle: usize,
    kind: u32,
    which: u32,
    owner: usize,
    group: usize,
    dacl: usize,
    sacl: usize,
    sd: usize,
) -> u32 {
    let handle = door_windows_abi::widen(handle);
    let owner = door_windows_abi::widen(owner);
    let group = door_windows_abi::widen(group);
    let dacl = door_windows_abi::widen(dacl);
    let sacl = door_windows_abi::widen(sacl);
    let sd = door_windows_abi::widen(sd);
    {
        error_code((|| {
            let d = match kind {
                SE_FILE_OBJECT => security::descriptor_of(&file_handle_security(handle)?),
                SE_REGISTRY_KEY => open_key_security(handle)?,
                SE_KERNEL_OBJECT => kernel_security(handle)?,
                _ => return Err(error::INVALID_PARAMETER),
            };
            hand_out(&selected(&d, which)?, [owner, group, dacl, sacl, sd])
        })())
    }
}
/// The descriptor a Set*SecurityInfo call describes with separate parts.
fn parts(which: u32, owner: u64, group: u64, dacl: u64, sacl: u64) -> Result<Descriptor, u32> {
    let mut d = Descriptor::default();
    if which & info::OWNER != 0 {
        d.owner = Some(read_sid(owner).map_err(|_| code::INVALID_OWNER)?);
    }
    if which & info::GROUP != 0 {
        d.group = Some(read_sid(group).map_err(|_| code::INVALID_PRIMARY_GROUP)?);
    }
    if which & info::DACL != 0 {
        d.control |= control::DACL_PRESENT;
        d.dacl = if dacl == 0 {
            None
        } else {
            Some(read_acl(dacl)?)
        };
    }
    if which & (info::SACL | info::LABEL) != 0 {
        d.control |= control::SACL_PRESENT;
        d.sacl = if sacl == 0 {
            None
        } else {
            Some(read_acl(sacl)?)
        };
    }
    Ok(d)
}
/// Apply `d`'s parts (`which`) to a file: storage keeps owner, entries and
/// label; a primary group has no place there and is accepted unchanged.
fn set_file(name: &[u16], which: u32, d: &Descriptor) -> Result<(), u32> {
    if which & info::SACL != 0 {
        return Err(code::PRIVILEGE_NOT_HELD);
    }
    let current = file_security(name)?;
    let next = security::apply(&current, which, d)?;
    let path = storage_path(name)?;
    storage(|s| s.set_security(&path, &next).map_err(win32))
}
/// Registry keys carry no descriptors of their own in Doors' hives: a
/// descriptor that matches what the hive enforces is accepted, any other
/// is ERROR_NOT_SUPPORTED.
fn set_key(current: &Descriptor, which: u32, d: &Descriptor) -> Result<(), u32> {
    let mut wanted = current.clone();
    if which & info::OWNER != 0 {
        wanted.owner.clone_from(&d.owner);
    }
    if which & info::GROUP != 0 {
        wanted.group.clone_from(&d.group);
    }
    if which & info::DACL != 0 {
        wanted.dacl.clone_from(&d.dacl);
    }
    if wanted.owner == current.owner && wanted.group == current.group && wanted.dacl == current.dacl
    {
        Ok(())
    } else {
        Err(error::NOT_SUPPORTED)
    }
}
#[allow(clippy::too_many_arguments)]
fn set_named(
    name: u64,
    kind: u32,
    which: u32,
    owner: u64,
    group: u64,
    dacl: u64,
    sacl: u64,
    wide: bool,
) -> u32 {
    error_code((|| {
        let name = text(name, wide)?;
        let d = parts(which, owner, group, dacl, sacl)?;
        match kind {
            SE_FILE_OBJECT => set_file(&name, which, &d),
            SE_REGISTRY_KEY => set_key(&key_security(&name)?, which, &d),
            _ => Err(error::INVALID_PARAMETER),
        }
    })())
}
#[unsafe(no_mangle)]
#[allow(clippy::too_many_arguments)]
pub extern "system" fn SetNamedSecurityInfoW(
    name: usize,
    kind: u32,
    which: u32,
    owner: usize,
    group: usize,
    dacl: usize,
    sacl: usize,
) -> u32 {
    let name = door_windows_abi::widen(name);
    let owner = door_windows_abi::widen(owner);
    let group = door_windows_abi::widen(group);
    let dacl = door_windows_abi::widen(dacl);
    let sacl = door_windows_abi::widen(sacl);
    set_named(name, kind, which, owner, group, dacl, sacl, true)
}
#[unsafe(no_mangle)]
#[allow(clippy::too_many_arguments)]
pub extern "system" fn SetNamedSecurityInfoA(
    name: usize,
    kind: u32,
    which: u32,
    owner: usize,
    group: usize,
    dacl: usize,
    sacl: usize,
) -> u32 {
    let name = door_windows_abi::widen(name);
    let owner = door_windows_abi::widen(owner);
    let group = door_windows_abi::widen(group);
    let dacl = door_windows_abi::widen(dacl);
    let sacl = door_windows_abi::widen(sacl);
    set_named(name, kind, which, owner, group, dacl, sacl, false)
}
#[unsafe(no_mangle)]
#[allow(clippy::too_many_arguments)]
pub extern "system" fn SetSecurityInfo(
    handle: usize,
    kind: u32,
    which: u32,
    owner: usize,
    group: usize,
    dacl: usize,
    sacl: usize,
) -> u32 {
    let handle = door_windows_abi::widen(handle);
    let owner = door_windows_abi::widen(owner);
    let group = door_windows_abi::widen(group);
    let dacl = door_windows_abi::widen(dacl);
    let sacl = door_windows_abi::widen(sacl);
    {
        error_code((|| {
            let d = parts(which, owner, group, dacl, sacl)?;
            match kind {
                SE_FILE_OBJECT => {
                    if which & info::SACL != 0 {
                        return Err(code::PRIVILEGE_NOT_HELD);
                    }
                    let next = security::apply(&file_handle_security(handle)?, which, &d)?;
                    if unsafe { DoorSetFileSecurity(handle, next.encode().as_ptr() as u64) } == 0 {
                        return Err(unsafe { RtlGetLastWin32Error() }.max(1));
                    }
                    Ok(())
                }
                SE_REGISTRY_KEY => set_key(&open_key_security(handle)?, which, &d),
                SE_KERNEL_OBJECT => set_key(&kernel_security(handle)?, which, &d),
                _ => Err(error::INVALID_PARAMETER),
            }
        })())
    }
}
/// A self-relative copy into the caller's buffer (GetFileSecurity,
/// RegGetKeySecurity).
fn copy_out(d: &Descriptor, out: u64, length: u32) -> Result<u32, u32> {
    let bytes = d.relative();
    if (length as usize) < bytes.len() {
        return Err(bytes.len() as u32);
    }
    write(out, &bytes).map_err(|_| 0u32)?;
    Ok(bytes.len() as u32)
}
fn get_file_security(name: u64, which: u32, out: u64, length: u32, needed: u64, wide: bool) -> i32 {
    let r = (|| {
        let name = text(name, wide)?;
        let d = selected(&named_descriptor(&name, SE_FILE_OBJECT)?, which)?;
        match copy_out(&d, out, length) {
            Ok(n) => write_u32(needed, n),
            Err(n) => {
                write_u32(needed, n)?;
                Err(error::INSUFFICIENT_BUFFER)
            }
        }
    })();
    last_error(r.err().unwrap_or(error::SUCCESS))
}
#[unsafe(no_mangle)]
pub extern "system" fn GetFileSecurityW(
    name: usize,
    which: u32,
    out: usize,
    length: u32,
    needed: usize,
) -> i32 {
    let name = door_windows_abi::widen(name);
    let out = door_windows_abi::widen(out);
    let needed = door_windows_abi::widen(needed);
    get_file_security(name, which, out, length, needed, true)
}
#[unsafe(no_mangle)]
pub extern "system" fn GetFileSecurityA(
    name: usize,
    which: u32,
    out: usize,
    length: u32,
    needed: usize,
) -> i32 {
    let name = door_windows_abi::widen(name);
    let out = door_windows_abi::widen(out);
    let needed = door_windows_abi::widen(needed);
    get_file_security(name, which, out, length, needed, false)
}
fn set_file_security(name: u64, which: u32, sd: u64, wide: bool) -> i32 {
    let r = (|| {
        let name = text(name, wide)?;
        set_file(&name, which, &read_descriptor(sd)?)
    })();
    last_error(r.err().unwrap_or(error::SUCCESS))
}
#[unsafe(no_mangle)]
pub extern "system" fn SetFileSecurityW(name: usize, which: u32, sd: usize) -> i32 {
    let name = door_windows_abi::widen(name);
    let sd = door_windows_abi::widen(sd);
    set_file_security(name, which, sd, true)
}
#[unsafe(no_mangle)]
pub extern "system" fn SetFileSecurityA(name: usize, which: u32, sd: usize) -> i32 {
    let name = door_windows_abi::widen(name);
    let sd = door_windows_abi::widen(sd);
    set_file_security(name, which, sd, false)
}
#[unsafe(no_mangle)]
pub extern "system" fn RegGetKeySecurity(key: usize, which: u32, out: usize, length: usize) -> u32 {
    let key = door_windows_abi::widen(key);
    let out = door_windows_abi::widen(out);
    let length = door_windows_abi::widen(length);
    {
        let r = (|| {
            let d = selected(&open_key_security(key)?, which)?;
            let have = read_u32(length)?;
            match copy_out(&d, out, have) {
                Ok(n) => write_u32(length, n),
                Err(n) => {
                    write_u32(length, n)?;
                    Err(error::INSUFFICIENT_BUFFER)
                }
            }
        })();
        r.err().unwrap_or(error::SUCCESS)
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn RegSetKeySecurity(key: usize, which: u32, sd: usize) -> u32 {
    let key = door_windows_abi::widen(key);
    let sd = door_windows_abi::widen(sd);
    {
        (|| set_key(&open_key_security(key)?, which, &read_descriptor(sd)?))()
            .err()
            .unwrap_or(error::SUCCESS)
    }
}

// ---- explicit access (aclapi) -----------------------------------------------------------------
/// TRUSTEE_W: pMultipleTrustee, MultipleTrusteeOperation, TrusteeForm,
/// TrusteeType, ptstrName (32 bytes on x64, 20 on x86).
const TRUSTEE: [layout::Field; 5] = {
    use layout::Field::*;
    [Pointer, U32, U32, U32, Pointer]
};
const TRUSTEE_IS_SID: u32 = 0;
const TRUSTEE_IS_NAME: u32 = 1;
fn trustee_at(trustee: u64, index: usize) -> u64 {
    trustee + layout::offset(&TRUSTEE, index) as u64
}
fn build_trustee(trustee: u64, form: u32, value: u64) -> Result<(), u32> {
    let mut b = [0u8; layout::size(&TRUSTEE)];
    layout::set(&mut b, &TRUSTEE, 2, u64::from(form));
    layout::set(&mut b, &TRUSTEE, 4, value);
    write(trustee, &b)
}
#[unsafe(no_mangle)]
pub extern "system" fn BuildTrusteeWithSidW(trustee: usize, sid: usize) {
    let trustee = door_windows_abi::widen(trustee);
    let sid = door_windows_abi::widen(sid);
    {
        let _ = build_trustee(trustee, TRUSTEE_IS_SID, sid);
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn BuildTrusteeWithSidA(trustee: usize, sid: usize) {
    let trustee = door_windows_abi::widen(trustee);
    let sid = door_windows_abi::widen(sid);
    {
        let _ = build_trustee(trustee, TRUSTEE_IS_SID, sid);
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn BuildTrusteeWithNameW(trustee: usize, name: usize) {
    let trustee = door_windows_abi::widen(trustee);
    let name = door_windows_abi::widen(name);
    {
        let _ = build_trustee(trustee, TRUSTEE_IS_NAME, name);
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn BuildTrusteeWithNameA(trustee: usize, name: usize) {
    let trustee = door_windows_abi::widen(trustee);
    let name = door_windows_abi::widen(name);
    {
        let _ = build_trustee(trustee, TRUSTEE_IS_NAME, name);
    }
}
fn trustee_sid(trustee: u64, wide: bool) -> Result<Sid, u32> {
    let form = read_u32(trustee_at(trustee, 2))?;
    let value = read_pointer(trustee_at(trustee, 4))?;
    match form {
        TRUSTEE_IS_SID => read_sid(value),
        TRUSTEE_IS_NAME => {
            let name = String::from_utf16_lossy(&text(value, wide)?);
            // "CURRENT_USER" names the caller.
            if name.eq_ignore_ascii_case("CURRENT_USER") {
                return Ok(Sid::user(identity().uid));
            }
            let computer = String::from_utf16_lossy(&computer_name());
            let (uid, me) = me();
            security::lookup_name(&name, &computer, (uid, &me))
                .map(|(sid, _, _)| sid)
                .ok_or(error::NONE_MAPPED)
        }
        _ => Err(error::INVALID_PARAMETER),
    }
}
/// SetEntriesInAcl: GRANT_ACCESS (1) adds to the trustee's allowed rights,
/// SET_ACCESS (2) replaces them, DENY_ACCESS (3) adds a denial,
/// REVOKE_ACCESS (4) removes the trustee's explicit grants (not its denials). The result is
/// in canonical order: explicit denials, explicit grants, inherited
/// entries.
fn set_entries(count: u32, entries: u64, old: u64, out: u64, wide: bool) -> u32 {
    (|| {
        let mut explicit: Vec<Ace> = Vec::new();
        let mut inherited: Vec<Ace> = Vec::new();
        if old != 0 {
            for a in read_acl(old)?.aces {
                if a.flags & ace::INHERITED != 0 {
                    inherited.push(a);
                } else {
                    explicit.push(a);
                }
            }
        }
        // EXPLICIT_ACCESS_W: grfAccessPermissions, grfAccessMode,
        // grfInheritance, then a TRUSTEE_W (48 bytes on x64, 32 on x86).
        const EXPLICIT_ACCESS: [layout::Field; 8] = {
            use layout::Field::*;
            [U32, U32, U32, Pointer, U32, U32, U32, Pointer]
        };
        let size = layout::size(&EXPLICIT_ACCESS) as u64;
        let trustee_at = layout::offset(&EXPLICIT_ACCESS, 3) as u64;
        for i in 0..u64::from(count) {
            let e = entries + i * size;
            let mask = read_u32(e)?;
            let mode = read_u32(e + 4)?;
            let inheritance = read_u32(e + 8)? as u8 & 0x1F;
            let sid = trustee_sid(e + trustee_at, wide)?;
            match mode {
                0 => (),
                1 => match explicit
                    .iter_mut()
                    .find(|a| a.kind == ace::ALLOWED && a.sid == sid && a.flags == inheritance)
                {
                    Some(a) => a.mask |= mask,
                    None => explicit.push(Ace::new(ace::ALLOWED, inheritance, mask, sid)),
                },
                2 => {
                    explicit.retain(|a| !(a.sid == sid && a.kind == ace::ALLOWED));
                    explicit.push(Ace::new(ace::ALLOWED, inheritance, mask, sid));
                }
                3 => explicit.push(Ace::new(ace::DENIED, inheritance, mask, sid)),
                // Grants (and audits) go; denials stay, as Windows.
                4 => explicit.retain(|a| !(a.sid == sid && a.kind != ace::DENIED)),
                _ => return Err(error::INVALID_PARAMETER),
            }
        }
        let mut aces: Vec<Ace> = explicit
            .iter()
            .filter(|a| a.kind == ace::DENIED)
            .cloned()
            .collect();
        aces.extend(explicit.into_iter().filter(|a| a.kind != ace::DENIED));
        aces.extend(inherited);
        let block = local_copy(&Acl { aces }.encode())?;
        write_pointer(out, block)
    })()
    .err()
    .unwrap_or(error::SUCCESS)
}
#[unsafe(no_mangle)]
pub extern "system" fn SetEntriesInAclW(count: u32, entries: usize, old: usize, out: usize) -> u32 {
    let entries = door_windows_abi::widen(entries);
    let old = door_windows_abi::widen(old);
    let out = door_windows_abi::widen(out);
    set_entries(count, entries, old, out, true)
}
#[unsafe(no_mangle)]
pub extern "system" fn SetEntriesInAclA(count: u32, entries: usize, old: usize, out: usize) -> u32 {
    let entries = door_windows_abi::widen(entries);
    let old = door_windows_abi::widen(old);
    let out = door_windows_abi::widen(out);
    set_entries(count, entries, old, out, false)
}

/// SetKernelObjectSecurity(handle, which, descriptor): a file handle's
/// security changes as SetSecurityInfo's. Doors' other objects are
/// reachable only through handles their owner transferred, under the
/// fixed descriptor GetKernelObjectSecurity reports; a valid descriptor
/// that grants the owner what it has is accepted without change.
#[unsafe(no_mangle)]
pub extern "system" fn SetKernelObjectSecurity(handle: usize, which: u32, sd: usize) -> i32 {
    let handle = door_windows_abi::widen(handle);
    let sd = door_windows_abi::widen(sd);
    let r = (|| {
        let d = read_descriptor(sd)?;
        if which & info::SACL != 0 {
            return Err(code::PRIVILEGE_NOT_HELD);
        }
        if let Ok(current) = file_handle_security(handle) {
            let next = security::apply(&current, which, &d)?;
            if unsafe { DoorSetFileSecurity(handle, next.encode().as_ptr() as u64) } == 0 {
                return Err(unsafe { RtlGetLastWin32Error() }.max(1));
            }
        }
        Ok(())
    })();
    last_error(r.err().unwrap_or(error::SUCCESS))
}

/// BuildExplicitAccessWithNameW: an EXPLICIT_ACCESS_W naming a trustee by
/// name (TRUSTEE_IS_NAME, TRUSTEE_IS_UNKNOWN, no multiple trustee).
#[unsafe(no_mangle)]
pub extern "system" fn BuildExplicitAccessWithNameW(
    entry: usize,
    name: usize,
    permissions: u32,
    mode: u32,
    inheritance: u32,
) {
    const EXPLICIT_ACCESS: [layout::Field; 8] = {
        use layout::Field::*;
        [U32, U32, U32, Pointer, U32, U32, U32, Pointer]
    };
    let mut b = [0u8; layout::size(&EXPLICIT_ACCESS)];
    for (index, value) in [
        (0, u64::from(permissions)),
        (1, u64::from(mode)),
        (2, u64::from(inheritance)),
        (5, u64::from(TRUSTEE_IS_NAME)),
        (7, door_windows_abi::widen(name)),
    ] {
        layout::set(&mut b, &EXPLICIT_ACCESS, index, value);
    }
    let _ = write(door_windows_abi::widen(entry), &b);
}
fn free_local(block: u64) {
    if block != 0 {
        unsafe {
            RtlFreeHeap(
                door_windows_abi::narrow(DoorGetProcessHeap()),
                0,
                door_windows_abi::narrow(block),
            )
        };
    }
}
/// BuildSecurityDescriptorW: `old` (or an empty descriptor) with the
/// owner and group trustees and the access entries merged into its DACL
/// as SetEntriesInAcl merges them, returned self-relative in a LocalAlloc
/// block. Audit entries (a SACL) are not supported.
#[unsafe(no_mangle)]
#[allow(clippy::too_many_arguments)]
pub extern "system" fn BuildSecurityDescriptorW(
    owner: usize,
    group: usize,
    access_count: u32,
    access: usize,
    audit_count: u32,
    _audits: usize,
    old: usize,
    size_out: usize,
    out: usize,
) -> u32 {
    let (owner, group, access) = (
        door_windows_abi::widen(owner),
        door_windows_abi::widen(group),
        door_windows_abi::widen(access),
    );
    let (old, size_out, out) = (
        door_windows_abi::widen(old),
        door_windows_abi::widen(size_out),
        door_windows_abi::widen(out),
    );
    (|| -> Result<(), u32> {
        if out == 0 || size_out == 0 || audit_count != 0 {
            return Err(error::INVALID_PARAMETER);
        }
        let mut d = if old != 0 {
            read_descriptor(old)?
        } else {
            Descriptor {
                control: 0,
                owner: None,
                group: None,
                sacl: None,
                dacl: None,
            }
        };
        if owner != 0 {
            d.owner = Some(trustee_sid(owner, true)?);
        }
        if group != 0 {
            d.group = Some(trustee_sid(group, true)?);
        }
        if access_count > 0 {
            let previous = match &d.dacl {
                Some(a) => local_copy(&a.encode())?,
                None => 0,
            };
            let mut acl = 0u64;
            let r = set_entries(
                access_count,
                access,
                previous,
                &mut acl as *mut u64 as u64,
                true,
            );
            free_local(previous);
            if r != error::SUCCESS {
                return Err(r);
            }
            let merged = read_acl(acl);
            free_local(acl);
            d.dacl = Some(merged?);
            d.control |= control::DACL_PRESENT;
        }
        let bytes = d.relative();
        let block = local_copy(&bytes)?;
        write_u32(size_out, bytes.len() as u32)?;
        write_pointer(out, block)
    })()
    .err()
    .unwrap_or(error::SUCCESS)
}
