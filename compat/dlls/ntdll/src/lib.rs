#![no_std]
//! Original native adapter; exported PE entry points exist only in guest builds.
#[cfg(all(feature = "guest-pe", target_os = "windows", not(test)))]
mod arch;
#[cfg(all(feature = "guest-pe", target_os = "windows", not(test)))]
mod completion;
#[cfg(all(feature = "guest-pe", target_os = "windows", not(test)))]
mod console;
#[cfg(all(feature = "guest-pe", target_os = "windows", not(test)))]
mod datafile;
#[cfg(all(feature = "guest-pe", target_os = "windows", not(test)))]
mod directories;
#[cfg(all(feature = "guest-pe", target_os = "windows", not(test)))]
mod environment;
#[cfg(all(feature = "guest-pe", target_os = "windows", not(test)))]
mod exception;
#[cfg(all(feature = "guest-pe", target_os = "windows", not(test)))]
mod file_nt;
#[cfg(all(feature = "guest-pe", target_os = "windows", not(test)))]
mod files;
#[cfg(all(feature = "guest-pe", target_os = "windows", not(test)))]
mod guest;
#[cfg(all(feature = "guest-pe", target_os = "windows", not(test)))]
mod heap;
#[cfg(all(feature = "guest-pe", target_os = "windows", not(test)))]
mod ldr;
#[cfg(all(feature = "guest-pe", target_os = "windows", not(test)))]
mod loader;
#[cfg(all(feature = "guest-pe", target_os = "windows", not(test)))]
mod memory;
#[cfg(all(feature = "guest-pe", target_os = "windows", not(test)))]
mod modules;
#[cfg(all(feature = "guest-pe", target_os = "windows", not(test)))]
mod named;
#[cfg(all(feature = "guest-pe", target_os = "windows", not(test)))]
mod native_api;
#[cfg(all(feature = "guest-pe", target_os = "windows", not(test)))]
mod net;
#[cfg(all(feature = "guest-pe", target_os = "windows", not(test)))]
mod pipes;
#[cfg(all(feature = "guest-pe", target_os = "windows", not(test)))]
mod process;
#[cfg(all(feature = "guest-pe", target_os = "windows", not(test)))]
mod sections;
#[cfg(all(
    feature = "guest-pe",
    target_os = "windows",
    target_arch = "x86",
    not(test)
))]
mod seh32;
#[cfg(all(
    feature = "guest-pe",
    target_os = "windows",
    target_arch = "x86_64",
    not(test)
))]
mod seh64;
#[cfg(all(feature = "guest-pe", target_os = "windows", not(test)))]
mod shapes;
#[cfg(all(
    feature = "guest-pe",
    target_os = "windows",
    target_arch = "x86_64",
    not(test)
))]
mod stacks;
#[cfg(all(feature = "guest-pe", target_os = "windows", not(test)))]
mod streams;
#[cfg(all(feature = "guest-pe", target_os = "windows", not(test)))]
mod sync;
#[cfg(all(feature = "guest-pe", target_os = "windows", not(test)))]
mod sysinfo;
#[cfg(all(feature = "guest-pe", target_os = "windows", not(test)))]
mod threads;
#[cfg(all(feature = "guest-pe", target_os = "windows", not(test)))]
mod tls;
#[cfg(all(
    feature = "guest-pe",
    target_os = "windows",
    target_arch = "x86",
    not(test)
))]
mod unwind;
#[cfg(all(feature = "guest-pe", target_os = "windows", not(test)))]
mod waits;
#[cfg(all(
    feature = "guest-pe",
    target_os = "windows",
    target_arch = "x86",
    not(test)
))]
mod wow64;

/// Splits console text into pieces of at most `limit` bytes that never cut
/// a UTF-8 character (a character longer than `limit` is its own piece).
pub fn utf8_chunks(text: &str, limit: usize) -> impl Iterator<Item = &str> {
    let mut rest = text;
    core::iter::from_fn(move || {
        if rest.is_empty() {
            return None;
        }
        let mut end = limit.min(rest.len());
        while !rest.is_char_boundary(end) {
            end -= 1;
        }
        if end == 0 {
            end = rest.chars().next().map_or(rest.len(), char::len_utf8);
        }
        let (piece, tail) = rest.split_at(end);
        rest = tail;
        Some(piece)
    })
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use std::vec::Vec;
    #[test]
    fn console_chunks_keep_utf8_characters_whole() {
        let text = "EDITOR_DIALOG name=doors-editor-\u{3bb}.txt\r\n";
        for limit in 1..text.len() + 2 {
            let pieces: Vec<&str> = utf8_chunks(text, limit).collect();
            assert_eq!(pieces.concat(), text);
            assert!(pieces.iter().all(|p| p.len() <= limit.max(2)));
        }
        assert_eq!(utf8_chunks("", 192).count(), 0);
    }
}
