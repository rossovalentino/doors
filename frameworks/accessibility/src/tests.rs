use crate::caption::{Announcer, caption};
use crate::mirror::{Applied, Mirror};
use crate::wire::{self, Message, WireError};
use crate::*;
use std::{string::String, vec, vec::Vec};

fn id(n: u64) -> NodeId {
    NodeId(n)
}
fn sample(version: u32) -> Tree {
    let mut b = TreeBuilder::new(7, version, Node::new(id(1), Role::Window).named("Gallery"));
    let mut save = Node::new(id(2), Role::Button).named("Save");
    save.states = States::FOCUSABLE;
    save.actions = Actions::INVOKE | Actions::FOCUS;
    save.bounds = Bounds::from_logical(10, 10, 120, 32);
    save.accelerator = "Ctrl+S".into();
    b.push(save).unwrap();
    let mut group = Node::new(id(3), Role::Group).named("Options");
    group.bounds = Bounds::from_logical(10, 50, 300, 100);
    b.push(group).unwrap();
    b.enter(id(3));
    let mut wifi = Node::new(id(4), Role::ToggleSwitch).named("Wi-Fi");
    wifi.states = States::FOCUSABLE | States::CHECKABLE;
    wifi.actions = Actions::TOGGLE | Actions::FOCUS;
    b.push(wifi).unwrap();
    let mut volume = Node::new(id(5), Role::Slider).named("Volume");
    volume.states = States::FOCUSABLE | States::HAS_RANGE | States::HORIZONTAL;
    volume.range = Some(RangeValue::integral(0, 100, 50, 1));
    volume.actions = Actions::INCREMENT | Actions::DECREMENT | Actions::SET_RANGE;
    b.push(volume).unwrap();
    let mut pw = Node::new(id(6), Role::Edit).named("Password");
    pw.states = States::FOCUSABLE | States::PROTECTED | States::EDITABLE;
    b.push(pw).unwrap();
    b.leave();
    b.finish(Some(id(2)))
}

#[test]
fn full_trees_roundtrip_and_validate_structure() {
    let t = sample(1);
    t.validate().unwrap();
    let bytes = wire::encode_full(&t).unwrap();
    assert_eq!(wire::header(&bytes).unwrap(), (wire::KIND_FULL, 7, 1, 0));
    match wire::decode(&bytes).unwrap() {
        Message::Full(d) => assert_eq!(d, t),
        other => panic!("{other:?}"),
    }
    // Reading order and nesting.
    let order: Vec<_> = t.walk().iter().map(|(n, d)| (n.id.0, *d)).collect();
    assert_eq!(order, [(1, 0), (2, 1), (3, 1), (4, 2), (5, 2), (6, 2)]);
    assert_eq!(
        t.ancestors(id(5))
            .iter()
            .map(|n| n.id.0)
            .collect::<Vec<_>>(),
        [3, 1]
    );
}

#[test]
fn malformed_messages_are_rejected_before_publication() {
    let t = sample(1);
    let bytes = wire::encode_full(&t).unwrap();
    // Every truncation fails.
    for n in 0..bytes.len() {
        assert!(wire::decode(&bytes[..n]).is_err(), "prefix {n}");
    }
    let mut extra = bytes.clone();
    extra.push(0);
    assert_eq!(wire::decode(&extra), Err(WireError::TrailingBytes));
    let mut magic = bytes.clone();
    magic[0] = b'X';
    assert_eq!(wire::decode(&magic), Err(WireError::BadMagic));
    let mut format = bytes.clone();
    format[4] = 9;
    assert_eq!(wire::decode(&format), Err(WireError::UnsupportedFormat));
    let mut reserved = bytes.clone();
    reserved[36] = 1;
    assert_eq!(wire::decode(&reserved), Err(WireError::Reserved));
    // Unknown role in the first record (offset 40 + 16).
    let mut role = bytes.clone();
    role[56] = 0xee;
    assert_eq!(wire::decode(&role), Err(WireError::UnknownRole));
    // Unknown state bits.
    let mut flags = bytes.clone();
    flags[60..64].copy_from_slice(&u32::MAX.to_le_bytes());
    assert_eq!(wire::decode(&flags), Err(WireError::UnknownFlags));

    // Structural faults in otherwise well-formed records.
    let mut cyc = sample(1);
    cyc.get_mut(id(3)).unwrap().children.push(id(1));
    assert!(cyc.validate().is_err());
    let mut orphan = sample(1);
    orphan
        .get_mut(id(3))
        .unwrap()
        .children
        .retain(|c| *c != id(4));
    assert_eq!(orphan.validate(), Err(WireError::InconsistentChildren));
    let mut focus = sample(1);
    focus.focus = Some(id(99));
    assert_eq!(focus.validate(), Err(WireError::UnknownFocus));
    // Password content can never be published.
    let mut leak = sample(1);
    leak.get_mut(id(6)).unwrap().value = "hunter2".into();
    assert!(wire::encode_full(&leak).is_err());
    // Control characters in names.
    let mut ctrl = sample(1);
    ctrl.get_mut(id(2)).unwrap().name = "Sa\u{1b}ve".into();
    assert_eq!(wire::encode_full(&ctrl), Err(WireError::BadString));
    // Limits.
    let mut long = sample(1);
    long.get_mut(id(2)).unwrap().name = "x".repeat(wire::MAX_NAME_BYTES + 1);
    assert_eq!(wire::encode_full(&long), Err(WireError::StringTooLong));
    let mut s = String::from("ab\u{e9}");
    assert!(wire::clamp(&mut s, 3));
    assert_eq!(s, "ab");
}

#[test]
fn depth_and_node_bounds_are_enforced() {
    let mut b = TreeBuilder::new(1, 1, Node::new(id(1), Role::Window));
    for n in 2..=(wire::MAX_DEPTH as u64 + 3) {
        b.push(Node::new(id(n), Role::Group)).unwrap();
        b.enter(id(n));
    }
    let deep = b.finish(None);
    assert_eq!(deep.validate(), Err(WireError::TooDeep));
    let mut wide = Tree::new(1, 1, Node::new(id(1), Role::List));
    for n in 0..wire::MAX_CHILDREN as u64 {
        wide.push(id(1), Node::new(id(n + 2), Role::ListItem))
            .unwrap();
    }
    assert_eq!(
        wide.push(id(1), Node::new(id(5000), Role::ListItem)),
        Err(WireError::TooManyChildren)
    );
}

#[test]
fn deltas_carry_only_changes_and_apply_to_their_exact_base() {
    let old = sample(1);
    let mut new = sample(2);
    new.focus = Some(id(4));
    new.get_mut(id(4)).unwrap().states |= States::CHECKED;
    let delta = new.diff(&old);
    assert_eq!(delta.upserts.len(), 1, "only the toggled switch changed");
    assert_eq!(delta.focus, Some(id(4)));
    let bytes = wire::encode_delta(&delta).unwrap();
    let mut mirror = Mirror::default();
    assert_eq!(mirror.apply_bytes(&bytes).unwrap(), Applied::NeedFull);
    mirror
        .apply_bytes(&wire::encode_full(&old).unwrap())
        .unwrap();
    assert_eq!(mirror.apply_bytes(&bytes).unwrap(), Applied::Current(2));
    assert_eq!(mirror.tree(7).unwrap(), &new);
    // Replaying the same delta is stale.
    assert_eq!(mirror.apply_bytes(&bytes).unwrap(), Applied::NeedFull);
    // A delta that would orphan nodes is rejected and leaves the tree intact.
    let bad = Delta {
        window: 7,
        base_version: 2,
        version: 3,
        focus: None,
        removed: vec![id(3)],
        upserts: vec![],
    };
    let before = mirror.tree(7).unwrap().clone();
    assert!(
        mirror
            .apply_bytes(&wire::encode_delta(&bad).unwrap())
            .is_err()
    );
    assert_eq!(mirror.tree(7).unwrap(), &before);
    // Removing a subtree with its parent's updated child list applies.
    let mut third = new.clone();
    third.version = 3;
    third
        .get_mut(id(3))
        .unwrap()
        .children
        .retain(|c| *c != id(6));
    let delta = Delta {
        window: 7,
        base_version: 2,
        version: 3,
        focus: Some(id(4)),
        removed: vec![id(6)],
        upserts: vec![third.get(id(3)).unwrap().clone()],
    };
    assert_eq!(
        mirror
            .apply_bytes(&wire::encode_delta(&delta).unwrap())
            .unwrap(),
        Applied::Current(3)
    );
    assert!(mirror.tree(7).unwrap().get(id(6)).is_none());
}

#[test]
fn redaction_and_placeholders_never_expose_protected_content() {
    let mut t = sample(1);
    // A provider bug cannot be published, but the broker also strips any
    // protected value from trees it holds.
    let pw = t.get_mut(id(6)).unwrap();
    pw.value = "secret".into();
    pw.selection = Some((0, 6));
    pw.states |= States::HAS_SELECTION;
    let bytes = wire::redact(&t).unwrap();
    let Message::Full(read) = wire::decode(&bytes).unwrap() else {
        panic!()
    };
    let node = read.get(id(6)).unwrap();
    assert!(node.value.is_empty() && node.selection.is_none());
    assert!(!bytes.windows(6).any(|w| w == b"secret"));
    let p = wire::placeholder(9, 4, id(1));
    let Message::Full(p) = wire::decode(&wire::encode_full(&p).unwrap()).unwrap() else {
        panic!()
    };
    assert_eq!(p.len(), 1);
    assert_eq!(p.root().name, "Secure surface");
    let one = wire::single(&t, id(5)).unwrap();
    assert_eq!(one.len(), 1);
    assert_eq!(one.root().name, "Volume");
}

#[test]
fn captions_read_like_narrator_and_follow_changes() {
    let t = sample(1);
    assert_eq!(caption(&t, t.get(id(2)).unwrap()), "Save, button, Ctrl+S");
    assert_eq!(
        caption(&t, t.get(id(4)).unwrap()),
        "Wi-Fi, toggle switch, Off"
    );
    assert_eq!(caption(&t, t.get(id(5)).unwrap()), "Volume, slider, 50");
    assert_eq!(
        caption(&t, t.get(id(6)).unwrap()),
        "Password, password edit, protected"
    );
    let mut a = Announcer::default();
    let said = a.observe(&t);
    assert_eq!(said.len(), 1);
    assert_eq!(said[0].text, "Save, button, Ctrl+S");
    assert!(a.observe(&t).is_empty(), "no change, no speech");
    let mut on = sample(2);
    on.focus = Some(id(4));
    assert_eq!(a.observe(&on)[0].text, "Wi-Fi, toggle switch, Off");
    on.get_mut(id(4)).unwrap().states |= States::CHECKED;
    assert_eq!(a.observe(&on)[0].text, "On");
    on.focus = Some(id(5));
    a.observe(&on);
    on.get_mut(id(5)).unwrap().range = Some(RangeValue::integral(0, 100, 51, 1));
    assert_eq!(a.observe(&on)[0].text, "51");
    // Live regions announce once per change.
    let mut bar = Node::new(id(9), Role::InfoBar).named("Update available");
    bar.states = States::LIVE_ASSERTIVE;
    on.push(id(1), bar).unwrap();
    let said = a.observe(&on);
    assert_eq!(said.len(), 1);
    assert!(said[0].assertive && said[0].text == "Update available");
    assert!(a.observe(&on).is_empty());
    assert!(a.history().len() <= Announcer::HISTORY);
}

#[test]
fn dump_is_stable_text() {
    let d = dump(&sample(1));
    assert!(d.starts_with("window \"Gallery\""));
    assert!(d.contains("  button \"Save\" [FOCUSABLE] {FOCUS INVOKE} *focus* @10,10 120x32"));
    assert!(d.contains("    slider \"Volume\" [FOCUSABLE HORIZONTAL HAS_RANGE]"));
    // The outline drops geometry, focus and visibility only.
    let mut t = sample(1);
    t.get_mut(id(2)).unwrap().states |= States::FOCUSED | States::OFFSCREEN;
    let o = outline(&t);
    assert!(
        o.contains("  button \"Save\" [FOCUSABLE] {FOCUS INVOKE}\n"),
        "{o}"
    );
    assert!(!o.contains(" @") && !o.contains("*focus*"));
    assert_eq!(
        outline_line("  group \"Options\" [OFFSCREEN] @10,50 300x100"),
        "  group \"Options\""
    );
    t.focus = Some(id(4));
    assert_eq!(outline(&t), o, "focus moves do not change the outline");
}

#[test]
fn action_codes_roundtrip_with_arguments() {
    for a in [
        Action::Focus,
        Action::Invoke,
        Action::Toggle,
        Action::Expand,
        Action::Collapse,
        Action::Select,
        Action::SetValue("Ciao".into()),
        Action::Increment,
        Action::Decrement,
        Action::ScrollIntoView,
        Action::ShowMenu,
        Action::Dismiss,
        Action::SetRange(-2500),
        Action::SetSelection(1, 4),
    ] {
        assert_eq!(Action::from_code(a.code(), &a.arg()), Some(a));
    }
    assert_eq!(Action::from_code(1, b"x"), None, "invoke takes no argument");
    assert_eq!(Action::from_code(6, &[0xff]), None);
    assert_eq!(Action::from_code(99, &[]), None);
}

fn retitled(mut t: Tree, window: u64, version: u32) -> Tree {
    t.window = window;
    t.version = version;
    t
}

#[test]
fn mirrors_track_windows_independently_and_resync_after_gaps() {
    let mut mirror = Mirror::default();
    let a1 = sample(1);
    let b1 = retitled(sample(1), 8, 1);
    mirror
        .apply_bytes(&wire::encode_full(&a1).unwrap())
        .unwrap();
    mirror
        .apply_bytes(&wire::encode_full(&b1).unwrap())
        .unwrap();
    assert_eq!(mirror.windows().collect::<Vec<_>>(), [7, 8]);
    // A delta for one window leaves the other untouched.
    let mut a2 = sample(2);
    a2.get_mut(id(5)).unwrap().range = Some(RangeValue::integral(0, 100, 75, 1));
    assert_eq!(
        mirror
            .apply_bytes(&wire::encode_delta(&a2.diff(&a1)).unwrap())
            .unwrap(),
        Applied::Current(2)
    );
    assert_eq!(mirror.tree(7).unwrap(), &a2);
    assert_eq!(mirror.tree(8).unwrap(), &b1);
    // A missed version is a gap: ask for a full tree, never guess.
    let mut a4 = sample(4);
    a4.focus = Some(id(5));
    let mut a3 = sample(3);
    a3.focus = Some(id(4));
    let gap = wire::encode_delta(&a4.diff(&a3)).unwrap();
    assert_eq!(mirror.apply_bytes(&gap).unwrap(), Applied::NeedFull);
    assert_eq!(mirror.tree(7).unwrap(), &a2, "unchanged by the gap");
    // Malformed bytes are rejected without touching the mirror.
    let full4 = wire::encode_full(&a4).unwrap();
    assert!(mirror.apply_bytes(&full4[..full4.len() - 1]).is_err());
    assert_eq!(mirror.tree(7).unwrap(), &a2);
    // The full snapshot resynchronizes; later deltas apply again.
    assert_eq!(mirror.apply_bytes(&full4).unwrap(), Applied::Current(4));
    let mut a5 = a4.clone();
    a5.version = 5;
    a5.get_mut(id(2)).unwrap().name = "Save as".into();
    let d = a5.diff(&a4);
    assert_eq!(d.upserts.len(), 1);
    assert_eq!(
        mirror
            .apply_bytes(&wire::encode_delta(&d).unwrap())
            .unwrap(),
        Applied::Current(5)
    );
    assert_eq!(mirror.tree(7).unwrap().get(id(2)).unwrap().name, "Save as");
    // A forgotten (closed) window needs a full tree again.
    mirror.forget(8);
    let b2 = retitled(sample(2), 8, 2);
    assert_eq!(
        mirror
            .apply_bytes(&wire::encode_delta(&b2.diff(&b1)).unwrap())
            .unwrap(),
        Applied::NeedFull
    );
    assert!(mirror.tree(8).is_none());
    // Diffing identical trees yields an empty delta that still advances.
    let same = retitled(a5.clone(), 7, 6).diff(&a5);
    assert!(same.is_empty());
    assert_eq!(
        mirror
            .apply_bytes(&wire::encode_delta(&same).unwrap())
            .unwrap(),
        Applied::Current(6)
    );
}

#[test]
fn focus_events_announce_entered_containers_once() {
    let mut t = sample(1);
    let mut dialog = Node::new(id(20), Role::Dialog).named("Save changes?");
    dialog.states = States::MODAL;
    t.push(id(1), dialog).unwrap();
    let mut ok = Node::new(id(21), Role::Button).named("Save");
    ok.states = States::FOCUSABLE;
    t.push(id(20), ok).unwrap();
    let mut cancel = Node::new(id(22), Role::Button).named("Cancel");
    cancel.states = States::FOCUSABLE;
    t.push(id(20), cancel).unwrap();
    let mut a = Announcer::default();
    a.observe(&t);
    // Entering the dialog names it first; moving inside it does not repeat it.
    t.focus = Some(id(21));
    let said = a.observe(&t);
    assert_eq!(said.len(), 1);
    assert_eq!(said[0].text, "Save changes?, dialog, Save, button");
    assert!(said[0].assertive);
    t.focus = Some(id(22));
    assert_eq!(a.observe(&t)[0].text, "Cancel, button");
    // State changes of the focused node are announced politely.
    t.get_mut(id(22)).unwrap().states |= States::DISABLED;
    let said = a.observe(&t);
    assert_eq!(
        (said[0].text.as_str(), said[0].assertive),
        ("dimmed", false)
    );
    // Focus in another window announces that window's element.
    let mut other = retitled(sample(1), 9, 1);
    other.focus = Some(id(4));
    let said = a.observe(&other);
    assert_eq!(
        (said[0].window, said[0].text.as_str()),
        (9, "Wi-Fi, toggle switch, Off")
    );
    // Offscreen live regions stay silent.
    let mut bar = Node::new(id(30), Role::InfoBar).named("Hidden status");
    bar.states = States::LIVE_POLITE | States::OFFSCREEN;
    other.push(id(1), bar).unwrap();
    assert!(a.observe(&other).is_empty());
}

/// M4: the validator walks with a visited set, so duplicate children, cycles
/// and detached subtrees are refused even when the visit count balances.
#[test]
fn hostile_trees_with_duplicates_cycles_or_detached_nodes_are_refused() {
    use std::collections::BTreeMap;
    let node = |n: u64, parent: Option<u64>, children: &[u64]| {
        let mut x = Node::new(id(n), Role::Group);
        x.parent = parent.map(id);
        x.children = children.iter().map(|c| id(*c)).collect();
        x
    };
    let tree = |nodes: Vec<Node>| {
        let map: BTreeMap<_, _> = nodes.into_iter().map(|n| (n.id, n)).collect();
        Tree::from_parts(7, 1, id(1), None, map)
    };
    let root = |children: &[u64]| node(1, None, children);
    // The critic's case: root [A, A] plus a detached self-cycle C.
    let t = tree(vec![
        root(&[2, 2]),
        node(2, Some(1), &[]),
        node(3, Some(3), &[3]),
    ]);
    assert_eq!(t.validate(), Err(WireError::InconsistentChildren));
    assert!(wire::encode_full(&t).is_err());
    let t = tree(vec![root(&[2, 2]), node(2, Some(1), &[])]);
    assert_eq!(t.validate(), Err(WireError::InconsistentChildren));
    let t = tree(vec![
        root(&[2]),
        node(2, Some(1), &[]),
        node(3, Some(3), &[3]),
    ]);
    assert_eq!(t.validate(), Err(WireError::Unreachable));
    let detached_pair = [node(3, Some(4), &[4]), node(4, Some(3), &[3])];
    let mut nodes = vec![root(&[2]), node(2, Some(1), &[])];
    nodes.extend(detached_pair.iter().cloned());
    assert_eq!(tree(nodes).validate(), Err(WireError::Unreachable));
    // Two parents list one child.
    let t = tree(vec![
        root(&[2, 3]),
        node(2, Some(1), &[4]),
        node(3, Some(1), &[4]),
        node(4, Some(2), &[]),
    ]);
    assert_eq!(t.validate(), Err(WireError::InconsistentChildren));
    // A delta grafting a detached cycle never applies; the tree is kept.
    let mut base = sample(1);
    let delta = Delta {
        window: 7,
        base_version: 1,
        version: 2,
        focus: Some(id(2)),
        removed: vec![],
        upserts: vec![node(40, Some(41), &[41]), node(41, Some(40), &[40])],
    };
    assert_eq!(base.apply(&delta), Err(WireError::Unreachable));
    assert_eq!(base, sample(1));
}
