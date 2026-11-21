//! Durable bytes and save acknowledgement are separate from editor history.
//! Every save is one atomic storage transaction: files whose old and new
//! contents fit 4 KiB use the checked replacement, larger ones (up to the
//! per-file cap) the staged whole-file replacement. No sibling copy exists.
use crate::document::Document;
use door_service_client::{
    Disposition, Error, File, LargeReplacement, READ, Replacement, SHARE_ALL, Storage, WRITE,
};
use door_service_protocol::{
    Code,
    files::{FileInfo, LARGE_REPLACE_BYTES, REPLACE_BYTES},
};
/// Which atomic replacement carries a save of `new` bytes over a file of
/// `old` bytes; None beyond the per-file cap.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Route {
    Checked,
    Staged,
}
pub fn route(old: u64, new: usize) -> Option<Route> {
    if new > LARGE_REPLACE_BYTES {
        None
    } else if new <= REPLACE_BYTES && old <= REPLACE_BYTES as u64 {
        Some(Route::Checked)
    } else {
        Some(Route::Staged)
    }
}
/// A candidate uploaded privately; nothing is published until `commit`.
#[derive(Debug)]
pub enum Prepared {
    Checked(Replacement),
    Staged(LargeReplacement),
}
pub fn prepare(
    storage: &mut Storage,
    file: &File,
    expected: FileInfo,
    bytes: &[u8],
) -> Result<Prepared, Error> {
    match route(expected.bytes, bytes.len()) {
        Some(Route::Checked) => storage
            .prepare_replace(file, expected, bytes)
            .map(Prepared::Checked),
        Some(Route::Staged) => {
            // Bulk upload: the service snapshots 64 KiB of a section per
            // request (65 requests for the largest file instead of ~19,600
            // messages). Without a section (no free mapping granule) the
            // bytes go by message.
            if !bytes.is_empty()
                && let Ok(section) = crate::surface::upload_section(bytes)
            {
                let prepared =
                    storage.prepare_large_replace_section(file, expected, section, bytes.len());
                let _ = door_sys::close(section);
                return prepared.map(Prepared::Staged);
            }
            storage
                .prepare_large_replace(file, expected, bytes)
                .map(Prepared::Staged)
        }
        None => Err(Error::Service(Code::Resources)),
    }
}
/// One durable payload/length/version transaction (or an uncertain error).
pub fn commit(storage: &mut Storage, prepared: Prepared) -> Result<FileInfo, Error> {
    match prepared {
        Prepared::Checked(r) => storage.commit_replace(r),
        Prepared::Staged(r) => storage.commit_large_replace(r),
    }
}
/// Save `bytes` to `path`: create it on first save, else replace the file
/// this document read (Changed if another writer replaced it). An unchanged
/// document whose last commit awaits close only re-verifies the identity.
pub fn save(
    storage: &mut Storage,
    path: &str,
    state: &mut SaveState,
    document: &mut Document,
    bytes: &[u8],
) -> Result<(), Error> {
    let file = storage.open(
        path,
        if state.identity().is_some() {
            Disposition::OpenExisting
        } else {
            Disposition::CreateNew
        },
        READ | WRITE,
        SHARE_ALL,
    )?;
    let saved = (|| {
        let expected = match state.identity() {
            Some(info) => info,
            None => {
                let info = storage.info(&file)?;
                state.created(info);
                info
            }
        };
        if !document.dirty() && state.outcome() == Outcome::ClosePending {
            return if storage.info(&file)? == expected {
                Ok(())
            } else {
                Err(Error::Service(Code::Changed))
            };
        }
        let prepared = prepare(storage, &file, expected, bytes)?;
        let previous = state.begin_commit();
        match commit(storage, prepared) {
            Ok(committed) => {
                state.committed(document, committed);
                Ok(())
            }
            Err(error) => {
                if let Error::Service(code) = error {
                    state.rejected(previous, code);
                }
                Err(error)
            }
        }
    })();
    let closed = storage.close(file);
    saved?;
    closed
}
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Outcome {
    #[default]
    Ready,
    ClosePending,
    Uncertain,
}
#[derive(Default)]
pub struct SaveState {
    identity: Option<FileInfo>,
    outcome: Outcome,
}
impl SaveState {
    pub fn identity(&self) -> Option<FileInfo> {
        self.identity
    }
    pub fn outcome(&self) -> Outcome {
        self.outcome
    }
    pub fn loaded(&mut self, info: FileInfo) {
        self.identity = Some(info);
        self.outcome = Outcome::Ready;
    }
    pub fn created(&mut self, info: FileInfo) {
        self.identity = Some(info);
    }
    pub fn attention(&self, document: &Document) -> bool {
        document.dirty() || self.outcome != Outcome::Ready
    }
    pub fn begin_commit(&mut self) -> Outcome {
        let previous = self.outcome;
        self.outcome = Outcome::Uncertain;
        previous
    }
    pub fn rejected(&mut self, previous: Outcome, code: Code) {
        // These owner preconditions cannot publish candidate bytes. Other errors
        // (including capacity/I/O/transport/malformed replies) may be uncertain.
        if matches!(
            code,
            Code::Changed
                | Code::Denied
                | Code::InvalidHandle
                | Code::InvalidRequest
                | Code::DeletePending
        ) {
            self.outcome = previous;
        }
    }
    pub fn committed(&mut self, document: &mut Document, info: FileInfo) {
        self.identity = Some(info);
        document.mark_saved();
        self.outcome = Outcome::ClosePending;
    }
    pub fn closed(&mut self) {
        if self.outcome == Outcome::ClosePending {
            self.outcome = Outcome::Ready;
        }
    }
}
#[cfg(test)]
mod tests {
    extern crate alloc;
    use super::*;
    fn info(version: u64) -> FileInfo {
        FileInfo {
            attributes: 0,
            bytes: 1,
            object: 27,
            version,
            directory: false,
        }
    }
    #[test]
    fn saves_route_to_one_atomic_replacement_by_size() {
        assert_eq!(route(0, 0), Some(Route::Checked));
        assert_eq!(route(4096, 4096), Some(Route::Checked));
        assert_eq!(route(4096, 4097), Some(Route::Staged));
        // Shrinking a large file is still a whole-file staged replacement.
        assert_eq!(route(4097, 10), Some(Route::Staged));
        assert_eq!(route(10, LARGE_REPLACE_BYTES), Some(Route::Staged));
        assert_eq!(route(10, LARGE_REPLACE_BYTES + 1), None);
    }
    #[test]
    fn known_commit_close_failure_then_undo_uses_new_durable_checkpoint() {
        let mut document = Document::new("a").unwrap();
        let mut save = SaveState::default();
        save.loaded(info(1));
        document.select_all();
        document.insert("b").unwrap();
        save.begin_commit();
        save.committed(&mut document, info(2));
        assert!(!document.dirty());
        assert!(save.attention(&document));
        document.undo();
        assert_eq!(document.to_text(), "a");
        assert!(document.dirty());
        assert!(save.attention(&document));
        assert_eq!(save.identity(), Some(info(2)));
        document.redo();
        assert!(!document.dirty());
        assert!(save.attention(&document));
        save.closed();
        assert!(!save.attention(&document));
    }
    #[test]
    fn uncertain_commit_then_undo_cannot_bypass_close_or_reopen_guard() {
        let mut document = Document::new("a").unwrap();
        let mut save = SaveState::default();
        save.loaded(info(1));
        document.select_all();
        document.insert("b").unwrap();
        save.begin_commit();
        document.undo();
        assert!(!document.dirty());
        assert!(save.attention(&document));
        save.closed();
        assert!(save.attention(&document));
        let previous = save.begin_commit();
        save.rejected(previous, Code::Changed);
        assert_eq!(save.outcome(), Outcome::Uncertain);
        assert!(save.attention(&document));
        save.committed(&mut document, info(2));
        save.closed();
        assert!(!save.attention(&document));
    }
    #[test]
    fn definite_rejection_preserves_previous_outcome_and_no_preparation_checkpoint() {
        let document = Document::new("").unwrap();
        let mut save = SaveState::default();
        save.loaded(info(1));
        for code in [
            Code::Changed,
            Code::Denied,
            Code::InvalidHandle,
            Code::InvalidRequest,
            Code::DeletePending,
        ] {
            let prior = save.begin_commit();
            save.rejected(prior, code);
            assert!(!save.attention(&document));
        }
        let prior = save.begin_commit();
        save.rejected(prior, Code::Io);
        assert!(save.attention(&document));
    }
}
