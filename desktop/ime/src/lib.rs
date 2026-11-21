#![no_std]
//! Door session input method (§53, §109): Korean 2-set Hangul composition,
//! Chinese Pinyin (syllable parser, dictionary candidates, frequency order,
//! paging, learning) and Japanese (romaji to kana, kana–kanji conversion with
//! segment editing). Engines are pure and host-tested; `session` maps
//! door.text-input records to engine calls; `main.rs` runs the process.
extern crate alloc;
pub mod dict;
pub mod engine;
pub mod hangul;
pub mod japanese;
pub mod kana;
pub mod pinyin;
pub mod session;
/// On-demand resource ids of the dictionaries (ADR 0018 resource sections,
/// `door_abi::platform::resource`): granted to the IME manifest only, and
/// optional (absent from builds without `assets/ime/*.dict`).
pub mod resources {
    pub const PINYIN: u64 = door_abi::platform::resource::PINYIN_DICTIONARY;
    pub const JAPANESE: u64 = door_abi::platform::resource::JAPANESE_DICTIONARY;
    /// Initfs paths of the dictionary resources.
    pub const PATHS: [&str; 2] = [
        door_abi::platform::resource::PATHS[PINYIN as usize - 1],
        door_abi::platform::resource::PATHS[JAPANESE as usize - 1],
    ];
}
