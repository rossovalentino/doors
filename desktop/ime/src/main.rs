#![no_std]
#![no_main]
//! The session IME process (Init starts one per signed-in session and
//! retires it at sign-out): maps its dictionaries from on-demand resource
//! sections, registers with the window server (manifest
//! `text_input::IME_MANIFEST`) and serves IME records until the session ends.
extern crate alloc;
use alloc::vec::Vec;
use door_abi::platform::{StartupV2, inherited};
use door_ime::dict::Dictionary;
use door_ime::session::{Action, Session, wire_preedit};
use door_keyboard_layouts::{Ime, methods};
use door_service_client::window::WindowClient;
use door_service_protocol::{BODY_BYTES, text_input as ti};
#[global_allocator]
static HEAP: door_rt::HeapAllocator = door_rt::HeapAllocator::new();
door_rt::entry_v2!(main);
/// Largest dictionary resource accepted (door_initfs::MAX_FILE).
const MAX_DICTIONARY: usize = 32 * 1024 * 1024;
/// Commit serial markers per IME process.
const MAX_COMMIT_MARKERS: u32 = 64;
/// Read-only view of a dictionary resource. An absent (not in this build),
/// oversized or malformed one leaves its engine composing without
/// candidates, which the IME reports at registration (Korean needs none).
fn dictionary(id: u64) -> (Dictionary, &'static str) {
    let data = match door_rt::resources::open(id) {
        Ok(data) => data,
        Err(door_abi::Status::NotFound) => return (Dictionary::empty(), "absent"),
        Err(_) => return (Dictionary::empty(), "unavailable"),
    };
    // The section is page-padded with zeros.
    let data = &data[..data.iter().rposition(|b| *b != 0).map_or(0, |i| i + 1)];
    if data.len() > MAX_DICTIONARY {
        return (Dictionary::empty(), "oversized");
    }
    match Dictionary::parse(data) {
        Ok(d) if !d.is_empty() => (d, "ready"),
        _ => (Dictionary::empty(), "malformed"),
    }
}
/// Method ids served by `engine` (bit = id).
fn method_bits(engine: Ime) -> u32 {
    methods::METHODS
        .iter()
        .filter(|m| m.ime == engine)
        .fold(0, |bits, m| bits | 1 << m.id)
}
fn run(startup: &StartupV2) -> door_service_client::Result<()> {
    // Parse before registering: until then keys reach applications directly.
    let (pinyin, pinyin_state) = dictionary(door_ime::resources::PINYIN);
    let (japanese, japanese_state) = dictionary(door_ime::resources::JAPANESE);
    let mut no_candidates = 0;
    if pinyin.is_empty() {
        no_candidates |= method_bits(Ime::Pinyin);
    }
    if japanese.is_empty() {
        no_candidates |= method_bits(Ime::Japanese);
    }
    door_rt::diagnostic(format_args!(
        "DOOR_P12 IME DICTIONARIES pinyin={pinyin_state} pinyin_entries={} japanese={japanese_state} japanese_entries={}\n",
        pinyin.len(),
        japanese.len()
    ));
    let registry = startup.base.inherited[inherited::REGISTRY].handle;
    let mut client = WindowClient::connect_with(registry, ti::CAP_IME)?;
    let mut session = Session::new(pinyin, japanese);
    let mut actions = Vec::new();
    let method = client.ime_register(
        methods::IME_METHODS | (no_candidates & methods::IME_METHODS) << ti::NO_CANDIDATES_SHIFT,
    )?;
    door_rt::diagnostic(format_args!("DOOR_P12 IME READY method={method}\n"));
    session.set_method(method, &mut actions);
    let mut idle = 0u32;
    // Commit markers (TEXT-01 synchronises on them), bounded so a long
    // session stays inside the diagnostic budget.
    let mut commits = 0u32;
    loop {
        for action in actions.drain(..) {
            let r = match action {
                Action::Commit { serial, text } => {
                    let r = client.ime_commit(serial, &text);
                    if r.is_ok() && commits < MAX_COMMIT_MARKERS {
                        commits += 1;
                        door_rt::diagnostic(format_args!(
                            "DOOR_P12 IME COMMIT n={commits} bytes={}\n",
                            text.len()
                        ));
                    }
                    r
                }
                Action::Preedit { serial, preedit } => {
                    let mut attrs = [ti::Attr::default(); ti::MAX_ATTRS];
                    let (len, n, cursor) = wire_preedit(&preedit, &mut attrs);
                    client.ime_preedit(
                        serial,
                        ti::Preedit {
                            text: &preedit.text[..len],
                            cursor,
                            attrs: &attrs[..n],
                        },
                    )
                }
                Action::Candidates { serial, page } => {
                    let items: Vec<&str> = page
                        .as_ref()
                        .map(|p| p.items.iter().map(|s| s.as_str()).collect())
                        .unwrap_or_default();
                    let p = page.as_ref();
                    let (selected, index, pages) = p.map_or((0, 0, 0), door_ime::engine::wire_page);
                    let r = client.ime_candidates(ti::Candidates {
                        serial,
                        selected,
                        page: index,
                        pages,
                        flags: if p.is_some_and(|p| p.vertical) {
                            ti::CANDIDATES_VERTICAL
                        } else {
                            0
                        },
                        items: &items,
                    });
                    // Pages are built to encode (`engine::bounds`); a refusal
                    // other than a stale serial is reported, never ignored.
                    if let Err(door_service_client::Error::Service(code)) = r
                        && code != door_service_protocol::Code::Changed
                    {
                        door_rt::diagnostic(format_args!(
                            "DOOR_P12 IME CANDIDATES REFUSED {code:?}\n"
                        ));
                    }
                    r
                }
                Action::KeyDone {
                    key_serial,
                    forward,
                } => client.ime_key_done(key_serial, forward),
                Action::Mode(mode) => client.ime_mode(mode),
            };
            // A stale serial (focus moved) is expected; transport errors end.
            if let Err(door_service_client::Error::Transport(e)) = r {
                return Err(door_service_client::Error::Transport(e));
            }
        }
        let mut buf = [0u8; BODY_BYTES];
        let n = client.text_input_poll(&mut buf)?;
        if n == 0 {
            // Typing bursts poll back to back; an idle IME sleeps a tick,
            // and after two quiet seconds 50 ms (well inside the 300 ms key
            // deadline), so a background IME costs the desktop little.
            idle = idle.saturating_add(1);
            if idle > 8 {
                let now = door_sys::time().unwrap_or(0);
                let _ = door_sys::sleep(now + if idle > 200 { 5 } else { 1 });
            } else {
                let _ = door_sys::yield_now();
            }
            continue;
        }
        idle = 0;
        for record in ti::records(&buf[..n]).flatten() {
            session.record(&record, &mut actions);
        }
    }
}
fn main(startup: StartupV2) -> i64 {
    if HEAP.initialize_with_bytes(48 * 1024 * 1024).is_err() {
        return -240;
    }
    match run(&startup) {
        Ok(()) => 0,
        Err(e) => {
            door_rt::diagnostic(format_args!("DOOR_IME FAILED {e:?}\n"));
            -241
        }
    }
}
#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    door_rt::panic()
}
