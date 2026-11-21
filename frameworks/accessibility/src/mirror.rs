//! Assistive-client copies of provider trees, kept coherent from full trees
//! and deltas. A gap in versions makes the mirror ask for a full snapshot
//! instead of guessing.
use crate::tree::Tree;
use crate::wire::{self, Message, WireError};
use alloc::collections::BTreeMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Applied {
    /// The window's tree is now at this version.
    Current(u32),
    /// A delta did not match the mirrored version; request a full tree.
    NeedFull,
}

#[derive(Default)]
pub struct Mirror {
    trees: BTreeMap<u64, Tree>,
}
impl Mirror {
    pub fn tree(&self, window: u64) -> Option<&Tree> {
        self.trees.get(&window)
    }
    pub fn windows(&self) -> impl Iterator<Item = u64> + '_ {
        self.trees.keys().copied()
    }
    pub fn forget(&mut self, window: u64) {
        self.trees.remove(&window);
    }
    pub fn clear(&mut self) {
        self.trees.clear();
    }
    /// Apply one encoded message. Malformed bytes are rejected without
    /// changing the mirror.
    pub fn apply_bytes(&mut self, bytes: &[u8]) -> Result<Applied, WireError> {
        self.apply(wire::decode(bytes)?)
    }
    pub fn apply(&mut self, message: Message) -> Result<Applied, WireError> {
        match message {
            Message::Full(tree) => {
                let version = tree.version;
                self.trees.insert(tree.window, tree);
                Ok(Applied::Current(version))
            }
            Message::Delta(delta) => match self.trees.get_mut(&delta.window) {
                Some(tree) if tree.version == delta.base_version => {
                    tree.apply(&delta)?;
                    Ok(Applied::Current(tree.version))
                }
                _ => Ok(Applied::NeedFull),
            },
        }
    }
}
