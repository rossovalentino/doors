//! Regression: the window server exhausted its heap as soon as an app window
//! published its semantic tree. The guest allocator (`door_rt::heap`) tracks
//! extents, and the broker retained every tree decoded (strings, child lists
//! and map nodes, about one allocation per node) and cloned it twice per
//! delta. A 40-node Settings tree took the server from about 60 live
//! allocations to 236 plus free gaps, past its then-fixed 256 extent
//! records, and a 7-byte allocation failed with 87 MB free.
//!
//! The broker now retains one encoding per tree, so what it holds no longer
//! grows with node count, and a delta decodes the tree only transiently.
//! A separate test binary: the counting allocator below is process-global
//! (counts are per thread).
use door_accessibility::{Actions, Node, NodeId, Role, States, Tree, TreeBuilder, wire};
use door_service_protocol::Code;
use door_service_protocol::accessibility as a;
use door_window::accessibility::{Broker, ClientKey, Context, Peer, Reply};
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

struct Counting;
thread_local! {
    static LIVE: Cell<isize> = const { Cell::new(0) };
    static PEAK: Cell<isize> = const { Cell::new(0) };
}
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let p = unsafe { System.alloc(layout) };
        if !p.is_null() {
            let _ = LIVE.try_with(|live| {
                let n = live.get() + 1;
                live.set(n);
                let _ = PEAK.try_with(|peak| peak.set(peak.get().max(n)));
            });
        }
        p
    }
    unsafe fn dealloc(&self, p: *mut u8, layout: Layout) {
        unsafe { System.dealloc(p, layout) };
        let _ = LIVE.try_with(|live| live.set(live.get() - 1));
    }
}
#[global_allocator]
static ALLOCATOR: Counting = Counting;

fn live() -> isize {
    LIVE.with(Cell::get)
}
/// Live allocations at the start of the measurement, and resets the peak.
fn mark() -> isize {
    let now = live();
    PEAK.with(|p| p.set(now));
    now
}
fn peak() -> isize {
    PEAK.with(Cell::get)
}

struct World {
    owners: Vec<(ClientKey, u64)>,
    focused: Option<u64>,
}
impl Context for World {
    fn owns(&self, client: ClientKey, window: u64) -> bool {
        self.owners.contains(&(client, window))
    }
    fn secure(&self, _window: u64) -> bool {
        false
    }
    fn focused_window(&self) -> Option<u64> {
        self.focused
    }
    fn snapshot(&mut self, _s: u64, _o: u64, _out: &mut [u8]) -> Result<(), Code> {
        Err(Code::Unsupported)
    }
}
fn app(pid: u64) -> Peer {
    Peer {
        key: (pid, 1, 1),
        assistive: false,
    }
}
const SETTINGS_WINDOW: u64 = 4_294_967_297;

/// A tree shaped like Settings' home page (`desktop/workspace/src/settings/
/// semantics.rs`): Back, the search box, the navigation rail, the heading
/// and card rows with descriptions. `rows` card rows; `selected` rail item;
/// `query` in the search box.
fn settings_like(window: u64, version: u32, rows: usize, selected: usize, query: &str) -> Tree {
    let mut root = Node::new(NodeId(1), Role::Window).named("Settings");
    root.bounds = door_accessibility::Bounds::from_logical(0, 0, 1129, 787);
    let mut b = TreeBuilder::new(window, version, root);
    let mut back = Node::new(NodeId(2), Role::Button).named("Back");
    back.states = States::FOCUSABLE | States::DISABLED;
    b.push(back).unwrap();
    let mut search = Node::new(NodeId(3), Role::Edit).named("Find a setting");
    search.states = States::FOCUSABLE | States::EDITABLE;
    search.actions = Actions::FOCUS | Actions::SET_VALUE;
    search.value = query.into();
    b.push(search).unwrap();
    let nav = b
        .push(Node::new(NodeId(4), Role::Navigation).named("Navigation"))
        .unwrap();
    b.enter(nav);
    for i in 0..13u64 {
        let mut item = Node::new(NodeId(100 + i), Role::ListItem).named("Bluetooth & devices");
        item.states = States::FOCUSABLE | States::SELECTABLE;
        item.states.set(States::SELECTED, i as usize == selected);
        item.actions = Actions::INVOKE;
        item.position = Some((i as u32 + 1, 13));
        b.push(item).unwrap();
    }
    b.leave();
    b.push(Node::new(NodeId(5), Role::Heading).named("Home"))
        .unwrap();
    for i in 0..rows as u64 {
        let mut card = Node::new(NodeId(1000 + i), Role::Button).named("Recommended settings");
        card.description = "Recent and commonly used settings".into();
        card.states = States::FOCUSABLE;
        card.actions = Actions::INVOKE;
        b.push(card).unwrap();
    }
    let focus = NodeId(100 + selected as u64);
    b.finish(Some(focus))
}
fn publish(br: &mut Broker, w: &mut World, peer: &Peer, bytes: &[u8]) -> Result<Reply, Code> {
    let window = u64::from_le_bytes(bytes[8..16].try_into().unwrap());
    let p = a::Publish::of_message(window, bytes)?;
    br.request(peer, a::PUBLISH_BEGIN, &p.encode(), &[], w)?;
    for (off, len) in a::chunks(bytes.len()) {
        let mut body = (off as u32).to_le_bytes().to_vec();
        body.extend_from_slice(&bytes[off..off + len]);
        br.request(peer, a::PUBLISH_CHUNK, &body, &[], w)?;
    }
    br.request(peer, a::PUBLISH_COMMIT, &[], &[], w)
}
/// Allocations one decoded copy of `tree` holds.
fn decoded_cost(tree: &Tree) -> isize {
    let bytes = wire::encode_full(tree).unwrap();
    let before = live();
    let decoded = wire::decode(&bytes).unwrap();
    let cost = live() - before;
    drop(decoded);
    cost
}

#[test]
fn published_trees_hold_a_few_allocations_however_many_nodes() {
    let settings = app(8);
    let mut w = World {
        owners: vec![(settings.key, SETTINGS_WINDOW)],
        focused: Some(SETTINGS_WINDOW),
    };
    let mut br = Broker::default();
    // Warm up the broker's own containers (trees map, events, staging).
    let warm = settings_like(SETTINGS_WINDOW, 1, 1, 0, "");
    publish(
        &mut br,
        &mut w,
        &settings,
        &wire::encode_full(&warm).unwrap(),
    )
    .unwrap();
    br.request(
        &settings,
        a::WITHDRAW,
        &SETTINGS_WINDOW.to_le_bytes(),
        &[],
        &mut w,
    )
    .unwrap();

    // The guest sequence: a full Settings tree after the first commit, then
    // deltas as the page settles (rail selection, search text).
    let mut published = settings_like(SETTINGS_WINDOW, 1, 22, 0, "");
    assert_eq!(published.len(), 40, "Settings home page size");
    let cost = decoded_cost(&published);
    assert!(
        cost >= 40,
        "a decoded 40-node tree costs {cost} allocations"
    );
    let full = wire::encode_full(&published).unwrap();
    // Everything live from here on, except `full`, belongs to the test.
    let base = mark() - 1;
    publish(&mut br, &mut w, &settings, &full).unwrap();
    drop(full);
    let retained = live() - base;
    assert!(
        retained <= 4,
        "the broker retains {retained} allocations for one 40-node tree (decoded: {cost})"
    );
    assert!(peak() - base <= cost + 24, "full commit peak");
    for version in 2..=6u32 {
        let next = settings_like(
            SETTINGS_WINDOW,
            version,
            22,
            version as usize % 13,
            &"q".repeat(version as usize),
        );
        let delta = wire::encode_delta(&next.diff(&published)).unwrap();
        let held = live() - base;
        let before = mark();
        let r = publish(&mut br, &mut w, &settings, &delta).unwrap();
        assert_eq!(r, Reply::Data(version.to_le_bytes().to_vec()));
        // The broker's share of the peak: what it retains plus one transient
        // decoded tree, `apply`'s working copy and the delta. The old broker
        // retained a decoded tree and added two clones of it (over 3x).
        let broker_peak = peak() - before + retained;
        assert!(
            broker_peak <= 2 * cost + 28,
            "delta {version}: broker peak {broker_peak} allocations (decoded tree costs {cost})"
        );
        drop((delta, r));
        assert_eq!(
            live() - base,
            held - 1,
            "delta {version} retains nothing more"
        );
        assert_eq!(br.tree(SETTINGS_WINDOW).as_ref(), Some(&next));
        published = next;
    }

    // Retention does not grow with node count: 400 nodes cost what 40 did.
    let big_window = SETTINGS_WINDOW + 1;
    w.owners.push((settings.key, big_window));
    let before = live();
    let big = settings_like(big_window, 1, 382, 3, "big");
    assert_eq!(big.len(), 400);
    let bytes = wire::encode_full(&big).unwrap();
    let held = live();
    publish(&mut br, &mut w, &settings, &bytes).unwrap();
    assert!(
        live() - held <= 4,
        "a 400-node tree is still a few allocations"
    );
    drop((big, bytes));
    let _ = before;

    // Six app windows (the desktop's app set) with Settings-sized trees:
    // dozens of retained allocations at most, not hundreds.
    let mut world = World {
        owners: (0..6).map(|i| (app(20 + i).key, 100 + i)).collect(),
        focused: Some(100),
    };
    let mut six = Broker::default();
    let start = live();
    for i in 0..6u64 {
        let t = settings_like(100 + i, 1, 22, 0, "");
        let bytes = wire::encode_full(&t).unwrap();
        drop(t);
        publish(&mut six, &mut world, &app(20 + i), &bytes).unwrap();
    }
    let retained = live() - start;
    assert!(
        retained <= 6 * 4 + 8,
        "six published windows retain {retained} allocations"
    );
}
