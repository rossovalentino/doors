//! A permanent removal always uses one captured observation, never a new selection.
use crate::dialog::Dialog;
use door_service_protocol::Code;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Observed {
    pub parent: u64,
    pub object: u64,
    pub version: u64,
}
#[derive(Default)]
pub struct Removal {
    pub dialog: Dialog,
    observed: Option<Observed>,
    refresh_required: bool,
}
impl Removal {
    pub fn begin(&mut self, observed: Observed) -> bool {
        if self.dialog.open()
            || self.refresh_required
            || observed.parent == 0
            || observed.object == 0
            || observed.version == 0
        {
            return false;
        }
        self.observed = Some(observed);
        self.dialog.show_focused(2, 1);
        true
    }
    /// Cancel and the one-button recovery notice never issue storage requests.
    pub fn choose(&mut self, choice: usize) -> Option<Observed> {
        if !self.dialog.open() || choice >= self.dialog.count() {
            return None;
        }
        if self.refresh_required || choice != 0 {
            self.cancel();
            return None;
        }
        self.observed
    }
    pub fn cancel(&mut self) {
        self.dialog.dismiss();
        self.observed = None;
    }
    /// Acknowledged removal is complete even if the subsequent listing fails.
    pub fn committed(&mut self) {
        self.cancel();
        self.refresh_required = true;
    }
    pub fn refresh_required(&self) -> bool {
        self.refresh_required
    }
    /// Endpoint replacement cannot preserve an old destructive intent. Keep an
    /// existing notice visible, but do not create a modal for an idle browser.
    pub fn connection_lost(&mut self) {
        if self.dialog.open() {
            self.rejected(None);
        } else {
            self.committed();
        }
    }
    pub fn refreshed(&mut self) {
        if !self.dialog.open() {
            self.refresh_required = false;
        }
    }
    /// Only explicit non-mutating owner preconditions permit the same retry.
    /// Conflict/pending/missing or uncertain outcomes require a fresh observation.
    pub fn rejected(&mut self, code: Option<Code>) {
        if matches!(
            code,
            Some(Code::Denied | Code::SharingViolation | Code::InvalidRequest)
        ) {
            // Never let a queued Enter turn a failed destructive action into an
            // unconsidered retry. The original observation remains available.
            self.dialog.dismiss();
            self.dialog.show_focused(2, 1);
            return;
        }
        self.refresh_required = true;
        self.observed = None;
        self.dialog.dismiss();
        self.dialog.show_focused(1, 0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn item(object: u64) -> Observed {
        Observed {
            parent: 17,
            object,
            version: 3,
        }
    }
    #[test]
    fn safe_default_and_escape_cancel_without_request() {
        let mut p = Removal::default();
        assert!(p.begin(item(28)));
        assert_eq!(p.dialog.focus(), Some(1));
        let choice = p.dialog.key(28, false).unwrap();
        assert_eq!(p.choose(choice), None);
        assert!(!p.dialog.open());
        assert!(p.begin(item(29)));
        let choice = p.dialog.key(1, false).unwrap();
        assert_eq!(p.choose(choice), None);
    }
    #[test]
    fn repeated_begin_and_definite_retry_keep_captured_identity() {
        let mut p = Removal::default();
        assert!(p.begin(item(28)));
        assert!(!p.begin(item(29)));
        assert_eq!(p.choose(0), Some(item(28)));
        p.rejected(Some(Code::SharingViolation));
        assert_eq!(p.dialog.focus(), Some(1));
        assert_eq!(p.dialog.key(15, false), None);
        assert_eq!(p.dialog.key(28, false), Some(0));
        assert_eq!(p.choose(0), Some(item(28)));
    }
    #[test]
    fn uncertain_and_stale_outcomes_require_successful_fresh_listing() {
        for code in [
            None,
            Some(Code::Changed),
            Some(Code::NotFound),
            Some(Code::DeletePending),
            Some(Code::Io),
        ] {
            let mut p = Removal::default();
            assert!(p.begin(item(28)));
            p.rejected(code);
            assert_eq!(p.dialog.count(), 1);
            assert!(p.refresh_required());
            p.refreshed(); // A listing cannot clear a still-visible notice.
            assert!(p.refresh_required());
            assert_eq!(p.choose(0), None);
            assert!(!p.begin(item(29)));
            p.refreshed();
            assert!(p.begin(item(29)));
        }
    }
    #[test]
    fn acknowledgement_clears_intent_before_fallible_refresh() {
        let mut p = Removal::default();
        assert!(p.begin(item(28)));
        assert_eq!(p.choose(0), Some(item(28)));
        p.committed();
        assert_eq!(p.choose(0), None);
        assert!(!p.begin(item(29)));
        p.refreshed();
        assert!(p.begin(item(29)));
    }
    #[test]
    fn reconnect_cannot_restore_old_destructive_intent_or_skip_fresh_listing() {
        for open in [false, true] {
            let mut p = Removal::default();
            if open {
                assert!(p.begin(item(28)));
            }
            p.connection_lost();
            assert!(p.refresh_required());
            assert_eq!(p.dialog.open(), open);
            assert_eq!(p.choose(0), None);
            assert!(!p.begin(item(29)));
            p.refreshed();
            assert!(p.begin(item(29)));
        }
    }
}
