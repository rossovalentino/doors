//! Start menu file search host (Windows 11 Start search "Documents" and
//! "Folders" groups). Init starts one windowless workspace process per
//! signed-in session in `mode::SEARCH_HOST` ([`host`]); it holds one
//! long-lived [`Index`] (`explorer_search`), keeps it fresh at a low listing
//! budget and answers the shell, which owns the search box and stays
//! storage-free. Protocol (`window::search`): the shell posts
//! SEARCH_QUERY(serial) after a debounce; the host reads the text with
//! SEARCH_QUERY_GET and answers with one SEARCH_RESULTS call per result (at
//! most `search::MAX_RESULTS`). SEARCH_OPEN / SEARCH_EXPLORE become
//! OPEN_DOCUMENT requests: text and source files open in Notes, images in
//! Preview, folders, "Open file location", other files and "Search in File
//! Explorer" in File Explorer. The host never opens windows itself.
extern crate alloc;
use crate::explorer_preview::Kind;
use crate::explorer_search::{Hit, HitKind, Index, Open, State};
use alloc::{string::String, vec::Vec};
use door_abi::Status;
use door_abi::platform::{StartupV2, inherited};
use door_service_client::{Error, Storage, window::WindowClient};
use door_service_protocol::{
    Code,
    window::{self as w, search},
};

/// Listing calls per host iteration while someone is searching and the
/// index is (re)building, and in the background otherwise.
pub const STEP_ACTIVE: usize = 32;
pub const STEP_IDLE: usize = 2;
/// Like the Windows Search service (Automatic, Delayed Start), the
/// background walk begins two minutes after the host starts, so sign-in
/// and the first app launches never compete with it. A query starts it at
/// once.
pub const START_DELAY_TICKS: u64 = 12_000;
/// A host iteration that took this long (the system is busy) is followed by
/// a pause four times as long before the next background step.
pub const BUSY_TICKS: u64 = 2;
/// A query this recent (monotonic ticks, 3 s) keeps the host in active mode.
pub const ACTIVE_TICKS: u64 = 300;
/// A finished index older than this (15 s) is rebuilt in the background
/// when a new query arrives; answers keep coming from it meanwhile.
pub const STALE_TICKS: u64 = 1500;
/// Without queries the index is still refreshed every 2 minutes.
pub const REFRESH_TICKS: u64 = 12_000;
/// While indexing, changed results are re-sent at most every 250 ms.
pub const RESEND_TICKS: u64 = 25;
/// Candidate matches ranked per query.
const CANDIDATES: usize = 256;
/// OPEN_DOCUMENT app commands.
pub const FILES: u32 = 1;
pub const NOTES: u32 = 2;
pub const PREVIEW: u32 = 10;

/// One answer: what the host sends for `serial`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Batch {
    pub serial: u32,
    pub state: u8,
    /// (kind, display name, containing folder in C:\ form), ranked.
    pub entries: Vec<(u8, String, String)>,
}
/// An OPEN_DOCUMENT request: app command and payload.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Request {
    pub app: u32,
    pub payload: Vec<u8>,
}
impl Request {
    /// File Explorer showing `open`; a location too long for the message
    /// opens without its selection, or not at all.
    pub fn explorer(open: &Open) -> Option<Self> {
        let mut payload = open.encode();
        if payload.len() > w::MAX_DOCUMENT
            && let Open::Location {
                folder,
                select: Some(_),
            } = open
        {
            payload = Open::Location {
                folder: folder.clone(),
                select: None,
            }
            .encode();
        }
        (payload.len() <= w::MAX_DOCUMENT).then_some(Self {
            app: FILES,
            payload,
        })
    }
    /// Opening a hit: text and source files in Notes, images in Preview,
    /// everything else (and `location`) in File Explorer.
    pub fn for_hit(hit: &Hit, location: bool) -> Option<Self> {
        if !location && hit.kind == HitKind::File && hit.path.len() <= w::MAX_DOCUMENT {
            let app = match Kind::of(&hit.name, false) {
                Kind::Text | Kind::Code => Some(NOTES),
                Kind::Image => Some(PREVIEW),
                _ => None,
            };
            if let Some(app) = app {
                return Some(Self {
                    app,
                    payload: hit.path.as_bytes().to_vec(),
                });
            }
        }
        Self::explorer(&if location {
            Open::file_location(hit)
        } else {
            Open::for_hit(hit)
        })
    }
}

pub struct Provider {
    index: Index,
    /// Background rebuild replacing `index` when it finishes.
    next: Option<Index>,
    built_at: Option<u64>,
    queried_at: Option<u64>,
    /// Current shell query (serial, text).
    query: Option<(u32, String)>,
    /// Hits last sent for `query`, in result order (SEARCH_OPEN indices).
    hits: Vec<Hit>,
    sent: Option<Batch>,
    /// A new serial must be answered even when nothing matches.
    pending: bool,
    grew: bool,
    sent_at: u64,
    /// The user's profile folder (`/users/<uid>`), home of `.door-recent`.
    home: String,
    /// Start > Recent items last sent (serial, items), for SEARCH_OPEN and
    /// "Remove from list".
    recent: Option<(u32, Vec<RecentItem>)>,
    /// No background walk before this tick (delayed start) until a query.
    delay_until: u64,
}
impl Default for Provider {
    fn default() -> Self {
        Self::new()
    }
}
impl Provider {
    pub fn new() -> Self {
        Self::with_index(Index::new())
    }
    /// A provider over a prepared index (host tests).
    pub fn with_index(index: Index) -> Self {
        let built_at = (index.state() != State::Building).then_some(0);
        Self {
            index,
            next: None,
            built_at,
            queried_at: None,
            query: None,
            hits: Vec::new(),
            sent: None,
            pending: false,
            grew: false,
            sent_at: 0,
            home: String::new(),
            recent: None,
            delay_until: 0,
        }
    }
    /// Delayed start: no background indexing before `until` unless queried.
    pub fn delay_until(mut self, until: u64) -> Self {
        self.delay_until = until;
        self
    }
    /// Whether the walk may advance now.
    pub fn indexing(&self, now: u64) -> bool {
        self.building() && (self.queried_at.is_some() || now >= self.delay_until)
    }
    pub fn with_home(mut self, home: String) -> Self {
        self.home = home;
        self
    }
    /// Start > Recent for `serial`: the newest `MAX_RECENT` items.
    pub fn recent_batch(&mut self, serial: u32, items: &[RecentItem]) -> Batch {
        let items: Vec<RecentItem> = items.iter().take(search::MAX_RECENT).cloned().collect();
        let batch = Batch {
            serial,
            state: search::COMPLETE,
            entries: items.iter().map(|i| entry(&recent_hit(i))).collect(),
        };
        self.recent = Some((serial, items));
        batch
    }
    fn recent_item(&self, serial: u32, index: u32) -> Option<&RecentItem> {
        let (s, items) = self.recent.as_ref()?;
        (*s == serial).then(|| items.get(index as usize)).flatten()
    }
    pub fn index(&self) -> &Index {
        &self.index
    }
    fn active(&self, now: u64) -> bool {
        self.queried_at
            .is_some_and(|at| now.saturating_sub(at) < ACTIVE_TICKS)
    }
    fn building(&self) -> bool {
        self.next.is_some() || self.index.state() == State::Building
    }
    /// Listing calls for this iteration.
    pub fn budget(&self, now: u64) -> usize {
        if self.active(now) {
            STEP_ACTIVE
        } else {
            STEP_IDLE
        }
    }
    /// Ticks to sleep when no event arrived: short while someone searches,
    /// 50 ms otherwise (background walk and idle polling).
    pub fn idle_ticks(&self, now: u64) -> u64 {
        if self.active(now) { 1 } else { 5 }
    }
    /// The shell posted a new query.
    pub fn set_query(&mut self, serial: u32, text: &str, now: u64) {
        if self.next.is_none()
            && self
                .built_at
                .is_some_and(|at| now.saturating_sub(at) >= STALE_TICKS)
        {
            // Files change; refresh the walk without dropping answers.
            self.next = Some(Index::new());
        }
        self.queried_at = Some(now);
        self.query = Some((serial, String::from(text.trim())));
        self.hits.clear();
        self.sent = None;
        self.pending = true;
    }
    /// Advance indexing by at most `budget(now)` listing calls.
    pub fn step(&mut self, storage: &mut Storage, now: u64) {
        if self.next.is_none()
            && self
                .built_at
                .is_some_and(|at| now.saturating_sub(at) >= REFRESH_TICKS)
        {
            self.next = Some(Index::new());
        }
        if !self.indexing(now) {
            return;
        }
        let calls = self.budget(now);
        if let Some(next) = self.next.as_mut() {
            next.step(storage, calls);
            if next.state() != State::Building {
                self.index = self.next.take().unwrap();
                self.built_at = Some(now);
                self.grew = true;
            }
        } else if self.index.state() == State::Building && self.index.step(storage, calls) {
            self.grew = true;
            if self.index.state() != State::Building {
                self.built_at = Some(now);
            }
        }
    }
    fn state(&self) -> u8 {
        match self.index.state() {
            State::Building => search::BUILDING,
            State::Complete => search::COMPLETE,
            State::Partial => search::PARTIAL,
        }
    }
    /// Results to send now: always once per serial, then again while the
    /// index grows (throttled) or finishes, when they changed.
    pub fn take_results(&mut self, now: u64) -> Option<Batch> {
        let (serial, text) = self.query.clone()?;
        let finished = self.index.state() != State::Building;
        let due = self.pending
            || self.grew && (finished || now >= self.sent_at.saturating_add(RESEND_TICKS));
        if !due {
            return None;
        }
        self.grew = false;
        let hits = rank(&self.index, &text);
        let batch = Batch {
            serial,
            state: self.state(),
            entries: hits.iter().map(entry).collect(),
        };
        if !self.pending && self.sent.as_ref() == Some(&batch) {
            return None;
        }
        self.pending = false;
        self.sent_at = now;
        self.hits = hits;
        self.sent = Some(batch.clone());
        Some(batch)
    }
    /// SEARCH_OPEN: the request for result `index`, and the File Explorer
    /// fallback used when its app cannot be started.
    /// Start > Recent items open the same way.
    pub fn open(
        &self,
        serial: u32,
        index: u32,
        location: bool,
    ) -> Option<(Request, Option<Request>)> {
        let recent = self.recent_item(serial, index).map(recent_hit);
        let hit = match recent.as_ref() {
            Some(hit) => hit,
            None => {
                self.query.as_ref().filter(|(s, _)| *s == serial)?;
                self.hits.get(index as usize)?
            }
        };
        let request = Request::for_hit(hit, location)?;
        let fallback = (request.app != FILES)
            .then(|| Request::explorer(&Open::file_location(hit)))
            .flatten();
        Some((request, fallback))
    }
    /// SEARCH_EXPLORE: This PC with the query run over every volume.
    pub fn explore(&self, serial: u32) -> Option<Request> {
        let (s, text) = self.query.as_ref()?;
        if *s != serial || text.is_empty() {
            return None;
        }
        Request::explorer(&Open::Search(text.clone()))
    }
    /// One shell event; returns a request to send (with its fallback).
    pub fn event(
        &mut self,
        client: &mut WindowClient,
        storage: &mut Storage,
        e: w::Event,
        now: u64,
    ) -> Result<Option<(Request, Option<Request>)>, Error> {
        match e.kind {
            w::SEARCH_RECENT => {
                // Missing file: empty; unreadable: the session's own list.
                let list = crate::recent::Recent::load(storage, &self.home);
                let batch = self.recent_batch(e.a, list.items());
                send(client, &batch)?;
                Ok(None)
            }
            w::SEARCH_RECENT_REMOVE => {
                if let Some(path) = self.recent_item(e.a, e.b).map(|i| i.path.clone()) {
                    let mut list = crate::recent::Recent::load(storage, &self.home);
                    list.remove(&path);
                    if let Err(error) = list.save(storage, &self.home) {
                        door_rt::diagnostic(format_args!(
                            "DOOR_P12 SEARCH_HOST RECENT_SAVE_FAILED {error:?}\n"
                        ));
                    }
                    if let Some((_, items)) = self.recent.as_mut() {
                        items.retain(|i| i.path != path);
                    }
                }
                Ok(None)
            }
            w::SEARCH_OPEN if self.recent_item(e.a, e.b).is_some() => {
                let opened = self.open(e.a, e.b, e.detail == 1);
                // Notes and Preview record their own opens; File Explorer
                // opening the item itself (not its location) is recorded here.
                if let Some((request, _)) = opened.as_ref()
                    && request.app == FILES
                    && e.detail != 1
                    && let Some(item) = self.recent_item(e.a, e.b)
                {
                    crate::recent::note_opened(storage, &item.path.clone(), item.kind);
                }
                Ok(opened)
            }
            w::SEARCH_QUERY => {
                let mut text = [0u8; search::MAX_QUERY];
                match client.search_query_get(e.a, &mut text) {
                    Ok(n) => {
                        let text =
                            core::str::from_utf8(&text[..n]).map_err(|_| Error::Malformed)?;
                        self.set_query(e.a, text, now);
                    }
                    // Superseded before it was read: a newer event follows.
                    Err(Error::Service(_)) => (),
                    Err(error) => return Err(error),
                }
                Ok(None)
            }
            w::SEARCH_OPEN => Ok(self.open(e.a, e.b, e.detail == 1)),
            w::SEARCH_EXPLORE => {
                if let Some(request) = self.explore(e.a) {
                    return Ok(Some((request, None)));
                }
                // Not read yet: ask the shell for the text of that serial.
                let mut text = [0u8; search::MAX_QUERY];
                let open = match client.search_query_get(e.a, &mut text) {
                    Ok(n) => core::str::from_utf8(&text[..n])
                        .ok()
                        .map(str::trim)
                        .filter(|t| !t.is_empty())
                        .map(|t| Open::Search(t.into())),
                    Err(Error::Service(_)) => None,
                    Err(error) => return Err(error),
                };
                let open = open.unwrap_or(Open::Location {
                    folder: "::ThisPC".into(),
                    select: None,
                });
                Ok(Request::explorer(&open).map(|r| (r, None)))
            }
            _ => Ok(None),
        }
    }
    /// Index a little and send due results.
    pub fn service(
        &mut self,
        storage: &mut Storage,
        client: &mut WindowClient,
        now: u64,
    ) -> Result<(), Error> {
        self.step(storage, now);
        let Some(batch) = self.take_results(now) else {
            return Ok(());
        };
        send(client, &batch)
    }
}
type RecentItem = crate::recent::Item;
/// A recent item as a search hit (same opening rules and row text).
fn recent_hit(item: &RecentItem) -> Hit {
    Hit {
        kind: match item.kind {
            crate::recent::Kind::Folder => HitKind::Folder,
            crate::recent::Kind::File => HitKind::File,
        },
        name: item.path.rsplit('/').next().unwrap_or_default().into(),
        path: item.path.clone(),
        display: item.display.clone(),
        bytes: 0,
        object: 0,
    }
}
/// Send a batch as SEARCH_RESULTS messages.
fn send(client: &mut WindowClient, batch: &Batch) -> Result<(), Error> {
    for message in messages(batch) {
        match client.search_result(message) {
            Ok(()) => (),
            // Stale serial or provider replaced: the shell dropped it.
            Err(Error::Service(Code::Denied | Code::InvalidRequest | Code::Unsupported)) => break,
            Err(error) => return Err(error),
        }
    }
    Ok(())
}
/// Send an OPEN_DOCUMENT request, falling back to File Explorer.
fn dispatch(
    client: &mut WindowClient,
    request: Request,
    fallback: Option<Request>,
) -> Result<(), Error> {
    match client.open_document(request.app, &request.payload) {
        Ok(()) => Ok(()),
        Err(Error::Service(code)) => {
            door_rt::diagnostic(format_args!(
                "DOOR_P12 SEARCH_HOST OPEN_FAILED app={} error={code:?}\n",
                request.app
            ));
            match fallback {
                Some(f) => match client.open_document(f.app, &f.payload) {
                    Ok(()) | Err(Error::Service(_)) => Ok(()),
                    Err(error) => Err(error),
                },
                None => Ok(()),
            }
        }
        Err(error) => Err(error),
    }
}
fn transient(error: &Error) -> bool {
    matches!(
        error,
        Error::Transport(
            Status::PeerClosed | Status::Timeout | Status::NotFound | Status::WouldBlock
        ) | Error::Service(Code::WouldBlock | Code::Closing)
    )
}
/// The windowless search host process (`mode::SEARCH_HOST`). It keeps its
/// index across display-service restarts and exits only when the shell
/// refuses it as the provider; Init retires it when the session ends.
pub fn host(startup: &StartupV2) -> Result<(), Error> {
    let registry = startup.base.inherited[inherited::REGISTRY].handle;
    let mut storage = loop {
        match Storage::connect(registry) {
            Ok(s) => break s,
            Err(error) if transient(&error) => door_sys::sleep(door_sys::time()? + 10)?,
            Err(error) => return Err(error),
        }
    };
    let mut provider = Provider::new()
        .with_home(alloc::format!("/users/{}", startup.identity.uid))
        .delay_until(door_sys::time()?.saturating_add(START_DELAY_TICKS));
    loop {
        let outcome = (|| -> Result<(), Error> {
            let mut client = WindowClient::connect_with(registry, w::CAP_SEARCH_PROVIDER)?;
            if !client.is_search_provider() {
                return Err(Error::Service(Code::Unsupported));
            }
            door_rt::diagnostic(format_args!(
                "DOOR_P12 SEARCH_HOST READY items={}\n",
                provider.index().len()
            ));
            loop {
                let now = door_sys::time()?;
                if storage.needs_reconnect()
                    && let Ok(s) = Storage::connect(registry)
                {
                    storage = s;
                }
                let polled = client.poll()?;
                if let Some(e) = polled
                    && let Some((request, fallback)) =
                        provider.event(&mut client, &mut storage, e, now)?
                {
                    dispatch(&mut client, request, fallback)?;
                }
                if !storage.needs_reconnect() {
                    provider.service(&mut storage, &mut client, now)?;
                }
                // Yield to the foreground: a slow iteration (busy system)
                // is followed by a proportionally longer pause.
                let spent = door_sys::time()?.saturating_sub(now);
                let pause = if spent >= BUSY_TICKS { spent * 4 } else { 0 };
                if polled.is_none() && !client.pending_events() || pause > 0 {
                    door_sys::sleep(now + spent + provider.idle_ticks(now) + pause)?;
                }
            }
        })();
        match outcome {
            Ok(()) => return Ok(()),
            Err(error) if transient(&error) => door_sys::sleep(door_sys::time()? + 10)?,
            Err(error) => return Err(error),
        }
    }
}
/// The wire messages for a batch: one per result, or one empty answer.
pub fn messages(batch: &Batch) -> Vec<search::Message<'_>> {
    let count = batch.entries.len().min(search::MAX_RESULTS) as u8;
    if count == 0 {
        return alloc::vec![search::Message {
            serial: batch.serial,
            state: batch.state,
            count: 0,
            entry: None,
        }];
    }
    batch.entries[..count as usize]
        .iter()
        .enumerate()
        .map(|(i, (kind, name, path))| search::Message {
            serial: batch.serial,
            state: batch.state,
            count,
            entry: Some((
                i as u8,
                search::Entry {
                    kind: *kind,
                    name,
                    path,
                },
            )),
        })
        .collect()
}
fn entry(hit: &Hit) -> (u8, String, String) {
    let kind = match hit.kind {
        HitKind::File => search::FILE,
        HitKind::Folder => search::FOLDER,
    };
    let folder = crate::explorer_path::display(&crate::explorer_path::parent(&hit.path, true));
    (
        kind,
        clip(&hit.name, search::MAX_NAME),
        elide(&folder, search::MAX_PATH),
    )
}
fn clip(text: &str, max: usize) -> String {
    if text.len() <= max {
        return text.into();
    }
    let mut end = max - 3;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    alloc::format!("{}...", &text[..end])
}
/// Long locations keep the drive and the innermost folders: "C:\...\a\b".
fn elide(path: &str, max: usize) -> String {
    if path.len() <= max {
        return path.into();
    }
    let head = path.find('\\').map_or(0, |i| i + 1);
    let keep = max.saturating_sub(head + 4);
    let mut start = path.len() - keep;
    while !path.is_char_boundary(start) {
        start += 1;
    }
    let tail = &path[start..];
    let tail = tail.find('\\').map_or(tail, |i| &tail[i + 1..]);
    alloc::format!("{}...\\{}", &path[..head], tail)
}
/// Windows-like ordering: exact name (or name without extension), then
/// prefix, then word start, then substring; shallower and shorter first.
pub fn rank(index: &Index, text: &str) -> Vec<Hit> {
    let query = text.trim().to_ascii_lowercase();
    let mut hits = index.query(text, CANDIDATES);
    hits.sort_by_cached_key(|h| {
        let name = h.name.to_ascii_lowercase();
        let stem = name.rsplit_once('.').map_or(name.as_str(), |(s, _)| s);
        let score = if name == query || stem == query {
            0
        } else if name.starts_with(&query) {
            1
        } else if name
            .match_indices(&query)
            .any(|(i, _)| name[..i].ends_with([' ', '-', '_', '.', '(']))
        {
            2
        } else {
            3
        };
        (score, h.path.matches('/').count(), h.name.len())
    });
    hits.truncate(search::MAX_RESULTS);
    hits
}

#[cfg(test)]
mod tests {
    use super::*;
    extern crate std;
    use door_service_client::{DirEntry, FileInfo};
    fn entry_named(name: &str, object: u64, directory: bool) -> DirEntry {
        let mut units = [0u16; door_service_protocol::files::NAME_UNITS];
        for (o, u) in units.iter_mut().zip(name.encode_utf16()) {
            *o = u;
        }
        DirEntry {
            times: Default::default(),
            cookie: object,
            name: units,
            name_len: name.encode_utf16().count(),
            info: FileInfo {
                attributes: 0,
                bytes: 1,
                object,
                version: 1,
                directory,
            },
        }
    }
    fn sample() -> Index {
        let mut index = Index::new();
        index.visit("/compat", 0, entry_named("Users", 2, true));
        index.visit("/users", 1, entry_named("1001", 3, true));
        index.visit("/users/1001", 2, entry_named("Documents", 4, true));
        index.visit(
            "/users/1001/Documents",
            3,
            entry_named("old budget report.txt", 5, false),
        );
        index.visit(
            "/users/1001/Documents",
            3,
            entry_named("Report.txt", 6, false),
        );
        index.visit("/users/1001/Documents", 3, entry_named("Reports", 7, true));
        index.visit(
            "/users/1001/Documents",
            3,
            entry_named("misreported.log", 8, false),
        );
        index.visit(
            "/users/1001/Documents",
            3,
            entry_named("report photo.png", 9, false),
        );
        index.visit(
            "/users/1001/Documents",
            3,
            entry_named("report.pdf", 10, false),
        );
        index
    }
    fn path(request: &Request) -> &str {
        core::str::from_utf8(&request.payload).unwrap()
    }
    #[test]
    fn ranks_like_start_and_answers_each_serial_once_with_bounded_messages() {
        let mut p = Provider::with_index(sample());
        assert_eq!(p.take_results(0), None, "no query yet");
        p.set_query(7, " report ", 0);
        let batch = p.take_results(0).unwrap();
        assert_eq!(batch.serial, 7);
        assert_eq!(batch.state, search::BUILDING);
        let names: Vec<&str> = batch.entries.iter().map(|e| e.1.as_str()).collect();
        assert_eq!(
            names,
            [
                "Report.txt",
                "report.pdf",
                "Reports",
                "report photo.png",
                "old budget report.txt",
                "misreported.log"
            ]
        );
        assert_eq!(batch.entries[2].0, search::FOLDER);
        assert_eq!(batch.entries[0].2, "C:\\Users\\1001\\Documents");
        // Unchanged: nothing is re-sent.
        assert_eq!(p.take_results(100), None);
        // Wire messages: one per result, decodable, inside the RPC body.
        let wire = messages(&batch);
        assert_eq!(wire.len(), 6);
        let mut body = [0u8; door_service_protocol::BODY_BYTES];
        for (i, m) in wire.iter().enumerate() {
            let n = search::encode(*m, &mut body).unwrap();
            let back = search::decode(&body[..n]).unwrap();
            assert_eq!(back.entry.unwrap().0, i as u8);
            assert_eq!(back.count, 6);
        }
        // A query with no match still answers (empty set).
        p.set_query(8, "zzz", 0);
        let empty = p.take_results(0).unwrap();
        assert!(empty.entries.is_empty());
        assert_eq!(messages(&empty)[0].entry, None);
    }
    #[test]
    fn results_open_in_notes_preview_or_file_explorer() {
        let mut p = Provider::with_index(sample());
        p.set_query(3, "report", 0);
        p.take_results(0).unwrap();
        // Text: Notes, with File Explorer as the fallback.
        let (text, fallback) = p.open(3, 0, false).unwrap();
        assert_eq!(
            (text.app, path(&text)),
            (NOTES, "/users/1001/Documents/Report.txt")
        );
        let fallback = fallback.unwrap();
        assert_eq!(fallback.app, FILES);
        assert_eq!(
            Open::decode(&fallback.payload),
            Some(Open::Location {
                folder: "/users/1001/Documents".into(),
                select: Some("Report.txt".into())
            })
        );
        // "Open file location" goes to File Explorer.
        let (location, none) = p.open(3, 0, true).unwrap();
        assert_eq!((location.app, none), (FILES, None));
        // A PDF has no app here: File Explorer at its location.
        let (pdf, none) = p.open(3, 1, false).unwrap();
        assert_eq!((pdf.app, none), (FILES, None));
        // A folder opens itself; an image opens in Preview.
        let (folder, _) = p.open(3, 2, false).unwrap();
        assert_eq!(
            Open::decode(&folder.payload),
            Some(Open::Location {
                folder: "/users/1001/Documents/Reports".into(),
                select: None
            })
        );
        let (image, _) = p.open(3, 3, false).unwrap();
        assert_eq!(
            (image.app, path(&image)),
            (PREVIEW, "/users/1001/Documents/report photo.png")
        );
        // Stale serials and indices open nothing.
        assert_eq!(p.open(2, 0, false), None);
        assert_eq!(p.open(3, 9, false), None);
        let explore = p.explore(3).unwrap();
        assert_eq!(
            (explore.app, Open::decode(&explore.payload)),
            (FILES, Some(Open::Search("report".into())))
        );
        assert_eq!(p.explore(4), None);
        // Over-long locations drop the selection rather than overflow.
        let long = Open::Location {
            folder: alloc::format!("/users/1001/{}", "d".repeat(300)),
            select: Some("x".into()),
        };
        assert_eq!(Request::explorer(&long), None);
        let deep = Open::Location {
            folder: alloc::format!("/users/1001/{}", "d".repeat(200)),
            select: Some("y".repeat(40)),
        };
        let request = Request::explorer(&deep).unwrap();
        assert!(request.payload.len() <= w::MAX_DOCUMENT);
        assert_eq!(
            Open::decode(&request.payload),
            Some(Open::Location {
                folder: alloc::format!("/users/1001/{}", "d".repeat(200)),
                select: None
            })
        );
    }
    #[test]
    fn recent_items_list_newest_six_and_open_like_results() {
        use crate::recent::{Kind as RecentKind, Recent};
        let mut list = Recent::default();
        for name in ["a.txt", "b.md", "c.pdf", "d.txt", "e.txt"] {
            list.record(
                &alloc::format!("/users/1001/notes/{name}"),
                RecentKind::File,
            );
        }
        list.record("/users/1001/Pictures/sunrise.png", RecentKind::File);
        list.record("/users/1001/Documents", RecentKind::Folder);
        let mut p = Provider::with_index(sample()).with_home("/users/1001".into());
        let batch = p.recent_batch(9, list.items());
        assert_eq!(
            (batch.serial, batch.state, batch.entries.len()),
            (9, search::COMPLETE, 6)
        );
        assert_eq!(
            batch.entries[0],
            (search::FOLDER, "Documents".into(), "C:\\Users\\1001".into())
        );
        assert_eq!(
            batch.entries[1],
            (
                search::FILE,
                "sunrise.png".into(),
                "C:\\Users\\1001\\Pictures".into()
            )
        );
        assert_eq!(batch.entries[5].1, "b.md");
        // Folder: File Explorer; image: Preview; text: Notes; location.
        let (folder, _) = p.open(9, 0, false).unwrap();
        assert_eq!(
            Open::decode(&folder.payload),
            Some(Open::Location {
                folder: "/users/1001/Documents".into(),
                select: None
            })
        );
        let (image, fallback) = p.open(9, 1, false).unwrap();
        assert_eq!(
            (image.app, path(&image)),
            (PREVIEW, "/users/1001/Pictures/sunrise.png")
        );
        assert_eq!(fallback.unwrap().app, FILES);
        assert_eq!(p.open(9, 2, false).unwrap().0.app, NOTES);
        let (location, _) = p.open(9, 2, true).unwrap();
        assert_eq!(
            Open::decode(&location.payload),
            Some(Open::Location {
                folder: "/users/1001/notes".into(),
                select: Some("e.txt".into())
            })
        );
        assert_eq!(p.open(9, 6, false), None);
        assert_eq!(p.open(8, 0, false), None, "another serial");
    }
    #[test]
    fn background_budget_and_refresh_policy() {
        let mut p = Provider::with_index(sample());
        // Building, nobody searching: low budget, slow polling.
        assert_eq!((p.budget(0), p.idle_ticks(0)), (STEP_IDLE, 5));
        // Delayed start: no background walk until the delay or a query.
        let mut delayed = Provider::with_index(Index::new()).delay_until(START_DELAY_TICKS);
        assert!(!delayed.indexing(0) && !delayed.indexing(START_DELAY_TICKS - 1));
        assert!(delayed.indexing(START_DELAY_TICKS));
        delayed.set_query(1, "a", 10);
        assert!(delayed.indexing(11), "a query starts the walk at once");
        p.set_query(1, "x", 100);
        assert_eq!((p.budget(150), p.idle_ticks(150)), (STEP_ACTIVE, 1));
        assert_eq!(p.budget(100 + ACTIVE_TICKS), STEP_IDLE);
        // A finished index idles; once stale, a query rebuilds it in the
        // background while answers keep coming from the old one.
        let mut partial = Index::for_roots(&["/"]);
        partial.visit(
            "/a",
            crate::explorer_search::MAX_DEPTH - 1,
            entry_named("deep", 1, true),
        );
        assert_eq!(partial.state(), State::Partial);
        let mut p = Provider::with_index(partial);
        assert_eq!((p.budget(10), p.idle_ticks(10)), (STEP_IDLE, 5));
        p.set_query(2, "deep", STALE_TICKS - 1);
        assert!(p.next.is_none(), "fresh enough");
        p.set_query(3, "deep", STALE_TICKS);
        assert!(p.next.is_some(), "stale index rebuilds on a query");
        let batch = p.take_results(STALE_TICKS).unwrap();
        assert_eq!(
            (batch.serial, batch.state, batch.entries.len()),
            (3, search::PARTIAL, 1)
        );
    }
    #[test]
    fn long_names_and_locations_fit_the_message_limits() {
        let long = "x".repeat(300);
        assert!(clip(&long, search::MAX_NAME).len() <= search::MAX_NAME);
        let path = alloc::format!("C:\\Users\\1001\\{}\\deep\\leaf", "segment\\".repeat(30));
        let short = elide(&path, search::MAX_PATH);
        assert!(short.len() <= search::MAX_PATH, "{short}");
        assert!(
            short.starts_with("C:\\...\\") && short.ends_with("\\deep\\leaf"),
            "{short}"
        );
        assert_eq!(elide("C:\\Users", search::MAX_PATH), "C:\\Users");
    }
    /// Host latency for the Start provider on a synthetic profile of about
    /// 5,000 files: index build (in-memory visits) and per-query ranking.
    #[test]
    fn query_latency_on_five_thousand_files() {
        use std::time::Instant;
        let words = [
            "report", "budget", "photo", "notes", "invoice", "draft", "plan", "summary", "letter",
            "data",
        ];
        let exts = ["txt", "md", "png", "jpg", "pdf", "rs", "zip", "csv"];
        let t = Instant::now();
        let mut index = Index::new();
        index.visit("/compat", 0, entry_named("Users", 1, true));
        index.visit("/users", 1, entry_named("1001", 2, true));
        let mut object = 10;
        for folder in 0..100 {
            let name = alloc::format!("{} folder {folder}", words[folder % words.len()]);
            index.visit("/users/1001", 2, entry_named(&name, object, true));
            object += 1;
            let parent = alloc::format!("/users/1001/{name}");
            for file in 0..50 {
                let n = alloc::format!(
                    "{} {} {file}.{}",
                    words[(folder + file) % words.len()],
                    words[file % 7],
                    exts[file % exts.len()]
                );
                index.visit(&parent, 3, entry_named(&n, object, false));
                object += 1;
            }
        }
        let build = t.elapsed();
        assert!(index.len() >= 5_000);
        let queries = [
            "rep",
            "report",
            "budget 1",
            "photo",
            "x",
            "notes folder",
            "zz",
            "draft plan",
            "summary 4",
            "csv",
            "letter",
            "invoice data",
        ];
        let mut times = Vec::new();
        for round in 0..20 {
            for q in queries {
                let t = Instant::now();
                let hits = rank(&index, q);
                times.push(t.elapsed().as_micros());
                assert!(hits.len() <= search::MAX_RESULTS, "{round} {q}");
            }
        }
        times.sort_unstable();
        let p50 = times[times.len() / 2];
        let p95 = times[times.len() * 95 / 100];
        std::eprintln!(
            "DOOR_SEARCH_LATENCY items={} build_us={} query_p50_us={p50} query_p95_us={p95}",
            index.len(),
            build.as_micros()
        );
        // Generous bound for unoptimized debug builds on a busy host.
        assert!(p95 < 250_000, "p95 {p95} us");
    }
}
