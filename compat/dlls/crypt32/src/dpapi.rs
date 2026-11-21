//! Data protection (DPAPI) blobs: CryptProtectData seals data under the
//! user's (or the computer's) 256-bit protection key with AES-256-GCM. The
//! blob starts like Windows' (version 1, the DPAPI provider GUID), then
//! records the flags and description; the header and the caller's optional
//! entropy are authenticated, so a blob only opens with the same key and
//! entropy. CryptProtectMemory encrypts in place with AES-256-CBC.
use aes::cipher::{BlockDecrypt, BlockEncrypt, generic_array::GenericArray};
use aes_gcm::aead::{Aead, KeyInit, Payload};
use alloc::vec::Vec;

/// {DF9D8CD0-1501-11D1-8C7A-00C04FC297EB}, the DPAPI provider.
pub const PROVIDER: [u8; 16] = [
    0xd0, 0x8c, 0x9d, 0xdf, 0x01, 0x15, 0xd1, 0x11, 0x8c, 0x7a, 0x00, 0xc0, 0x4f, 0xc2, 0x97, 0xeb,
];
/// CRYPTPROTECT_LOCAL_MACHINE.
pub const LOCAL_MACHINE: u32 = 0x4;
const VERSION: u32 = 1;
const NONCE: usize = 12;
/// Version, provider, flags and description length.
const FIXED: usize = 4 + 16 + 4 + 4;

/// A sealed blob of `plain` with `description` (UTF-16, no terminator).
pub fn seal(
    key: &[u8; 32],
    nonce: [u8; NONCE],
    flags: u32,
    description: &[u16],
    entropy: &[u8],
    plain: &[u8],
) -> Option<Vec<u8>> {
    let mut blob = Vec::with_capacity(FIXED + 2 * description.len() + NONCE + plain.len() + 16);
    blob.extend_from_slice(&VERSION.to_le_bytes());
    blob.extend_from_slice(&PROVIDER);
    blob.extend_from_slice(&(flags & LOCAL_MACHINE).to_le_bytes());
    blob.extend_from_slice(&(2 * description.len() as u32).to_le_bytes());
    blob.extend(description.iter().flat_map(|u| u.to_le_bytes()));
    let header = blob.len();
    blob.extend_from_slice(&nonce);
    let mut aad = Vec::with_capacity(header + entropy.len());
    aad.extend_from_slice(&blob[..header]);
    aad.extend_from_slice(entropy);
    let cipher = aes_gcm::Aes256Gcm::new(GenericArray::from_slice(key));
    let sealed = cipher
        .encrypt(
            GenericArray::from_slice(&nonce),
            Payload {
                msg: plain,
                aad: &aad,
            },
        )
        .ok()?;
    blob.extend_from_slice(&sealed);
    Some(blob)
}
/// The flags a blob was sealed with (which key opens it), if it is one.
pub fn flags(blob: &[u8]) -> Option<u32> {
    if blob.len() < FIXED + NONCE + 16
        || blob[..4] != VERSION.to_le_bytes()
        || blob[4..20] != PROVIDER
    {
        return None;
    }
    Some(u32::from_le_bytes(blob[20..24].try_into().ok()?))
}
/// The description and data of a blob sealed with `key` and `entropy`.
pub fn open(key: &[u8; 32], entropy: &[u8], blob: &[u8]) -> Option<(Vec<u16>, Vec<u8>)> {
    flags(blob)?;
    let length = u32::from_le_bytes(blob[24..28].try_into().ok()?) as usize;
    let header = FIXED.checked_add(length)?;
    if !length.is_multiple_of(2) || blob.len() < header + NONCE + 16 {
        return None;
    }
    let description = blob[FIXED..header]
        .chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .collect();
    let mut aad = Vec::with_capacity(header + entropy.len());
    aad.extend_from_slice(&blob[..header]);
    aad.extend_from_slice(entropy);
    let cipher = aes_gcm::Aes256Gcm::new(GenericArray::from_slice(key));
    let plain = cipher
        .decrypt(
            GenericArray::from_slice(&blob[header..header + NONCE]),
            Payload {
                msg: &blob[header + NONCE..],
                aad: &aad,
            },
        )
        .ok()?;
    Some((description, plain))
}
/// CryptProtectMemory's cipher: AES-256-CBC over whole 16-byte blocks
/// (the length must be a multiple of 16), IV zero.
pub fn memory(key: &[u8; 32], data: &mut [u8], encrypt: bool) -> bool {
    if !data.len().is_multiple_of(16) {
        return false;
    }
    let cipher = aes::Aes256::new(GenericArray::from_slice(key));
    let mut chain = [0u8; 16];
    for block in data.chunks_exact_mut(16) {
        if encrypt {
            for (b, c) in block.iter_mut().zip(&chain) {
                *b ^= c;
            }
            cipher.encrypt_block(GenericArray::from_mut_slice(block));
            chain.copy_from_slice(block);
        } else {
            let mut saved = [0u8; 16];
            saved.copy_from_slice(block);
            cipher.decrypt_block(GenericArray::from_mut_slice(block));
            for (b, c) in block.iter_mut().zip(&chain) {
                *b ^= c;
            }
            chain = saved;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn blobs_open_only_with_their_key_and_entropy() {
        let key = [7u8; 32];
        let description: Vec<u16> = "Chromium Safe Storage".encode_utf16().collect();
        let blob = seal(&key, [1; 12], 0, &description, b"salt", b"secret bytes").unwrap();
        assert_eq!(&blob[4..20], &PROVIDER);
        assert_eq!(flags(&blob), Some(0));
        let (d, plain) = open(&key, b"salt", &blob).unwrap();
        assert_eq!(d, description);
        assert_eq!(plain, b"secret bytes");
        assert!(open(&key, b"other", &blob).is_none());
        assert!(open(&[8; 32], b"salt", &blob).is_none());
        let mut tampered = blob.clone();
        tampered[30] ^= 1;
        assert!(open(&key, b"salt", &tampered).is_none());
        assert!(open(&key, b"salt", &blob[..20]).is_none());
        let machine = seal(&key, [2; 12], LOCAL_MACHINE | 1, &[], &[], b"").unwrap();
        assert_eq!(flags(&machine), Some(LOCAL_MACHINE));
        assert_eq!(open(&key, &[], &machine).unwrap().1, b"");
    }
    #[test]
    fn memory_protection_round_trips_whole_blocks() {
        let key = [3u8; 32];
        let mut data = *b"0123456789abcdef0123456789abcdef";
        assert!(memory(&key, &mut data, true));
        assert_ne!(&data, b"0123456789abcdef0123456789abcdef");
        // CBC: equal plaintext blocks encrypt differently.
        assert_ne!(data[..16], data[16..]);
        assert!(memory(&key, &mut data, false));
        assert_eq!(&data, b"0123456789abcdef0123456789abcdef");
        assert!(!memory(&key, &mut [0u8; 15], true));
    }
}
