//! Intent survives failed saves/reloads. Closing approval traps later input
//! until the Window server delivers CLOSE, so queued edits cannot be discarded.
use crate::dialog::Dialog;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Intent {
    Close,
    Reload,
}
#[derive(Default)]
pub struct Prompt {
    pub dialog: Dialog,
    intent: Option<Intent>,
    approved_close: bool,
    request_close: bool,
}
impl Prompt {
    pub fn intent(&self) -> Option<Intent> {
        self.intent
    }
    pub fn blocked(&self) -> bool {
        self.dialog.open() || self.approved_close
    }
    pub fn show(&mut self, intent: Intent) {
        if self.blocked() {
            return;
        }
        self.intent = Some(intent);
        self.dialog.show(3);
    }
    pub fn cancel(&mut self) {
        self.dialog.dismiss();
        self.intent = None;
    }
    pub fn completed(&mut self) {
        if self.intent == Some(Intent::Close) {
            self.approved_close = true;
            self.request_close = true;
        }
        self.cancel();
    }
    pub fn take_close(&mut self) -> bool {
        core::mem::take(&mut self.request_close)
    }
    pub fn close_failed(&mut self) {
        self.approved_close = false;
        self.request_close = false;
        self.intent = Some(Intent::Close);
        self.dialog.show(3);
    }
    /// An old server's close approval cannot be consumed by a new window.
    /// Restore a visible choice instead of silently discarding or trapping input.
    pub fn window_recreated(&mut self) -> bool {
        if !self.approved_close {
            return false;
        }
        self.close_failed();
        true
    }
    pub fn accepts_close(&self, attention: bool) -> bool {
        self.approved_close || (!self.blocked() && !attention)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn repeated_close_keeps_original_intent_and_failed_actions_keep_guard() {
        let mut p = Prompt::default();
        p.show(Intent::Reload);
        p.show(Intent::Close);
        assert_eq!(p.intent(), Some(Intent::Reload));
        assert!(!p.accepts_close(true));
        // No completed call on failure, including an uncertain save or failed load.
        assert!(p.blocked());
        assert!(!p.take_close());
        p.cancel();
        assert!(!p.blocked());
        assert!(!p.accepts_close(true));
        assert!(p.accepts_close(false));
    }
    #[test]
    fn approved_close_traps_input_through_the_async_close_drain() {
        let mut p = Prompt::default();
        p.show(Intent::Close);
        p.completed();
        assert!(p.take_close());
        assert!(!p.take_close());
        assert!(p.blocked());
        assert!(!p.dialog.open());
        assert!(p.accepts_close(true));
        p.show(Intent::Reload);
        assert_eq!(p.intent(), None);
    }
    #[test]
    fn successful_reload_releases_guard_without_requesting_close() {
        let mut p = Prompt::default();
        p.show(Intent::Reload);
        p.completed();
        assert!(!p.blocked());
        assert!(!p.take_close());
    }
    #[test]
    fn failed_close_request_restores_a_usable_prompt_without_losing_intent() {
        let mut p = Prompt::default();
        p.show(Intent::Close);
        p.completed();
        assert!(p.take_close());
        p.close_failed();
        assert!(p.dialog.open());
        assert!(p.blocked());
        assert_eq!(p.intent(), Some(Intent::Close));
        assert!(!p.accepts_close(false));
        assert!(!p.take_close());
        p.cancel();
        assert!(!p.blocked());
    }
    #[test]
    fn successful_close_reply_then_service_loss_restores_visible_choice() {
        let mut p = Prompt::default();
        p.show(Intent::Close);
        p.completed();
        assert!(p.take_close()); // The old server accepted the request.
        assert!(p.blocked() && !p.dialog.open());
        assert!(p.window_recreated());
        assert!(p.dialog.open());
        assert!(!p.take_close());
        assert!(!p.accepts_close(false));
        assert_eq!(p.intent(), Some(Intent::Close));
        p.cancel();
        assert!(!p.blocked());
        assert!(!p.window_recreated());
        p.show(Intent::Reload);
        assert!(!p.window_recreated());
        assert_eq!(p.intent(), Some(Intent::Reload));
    }
}
