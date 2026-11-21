//! OPEN_DOCUMENT authority: who may ask the server to open a document in a
//! catalog app (launch or activate File Explorer, Notes or Preview and post
//! it DOCUMENT_OPEN).
use door_service_protocol::Code;

/// The session's search provider (Start search) and clients that negotiated
/// CAP_DESKTOP_CONTROLS (File Explorer's Open and Open with) may send
/// OPEN_DOCUMENT; every other client is refused with `Denied`.
pub const fn authorize(search_provider: bool, desktop_controls: bool) -> Result<(), Code> {
    if search_provider || desktop_controls {
        Ok(())
    } else {
        Err(Code::Denied)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn search_provider_and_explorer_may_open_documents_but_other_clients_may_not() {
        assert_eq!(authorize(true, false), Ok(()), "Start search");
        assert_eq!(authorize(false, true), Ok(()), "File Explorer");
        assert_eq!(authorize(true, true), Ok(()));
        assert_eq!(authorize(false, false), Err(Code::Denied), "ordinary");
    }
}
