//! Host tests: schema, encodings, propose/commit, propagation between two
//! registries sharing one storage, restart, interrupted writes, corrupt
//! values and storage, machine scope, apply timing and dependencies.
mod mock;
use door_service_client::{RegistryRoot, Setting};
use door_service_protocol::desktop::{ContrastTheme, Controls, ShellLayout};
use door_settings_map::Page;
use door_settings_registry::schema::validate;
use door_settings_registry::{
    Apply, Authority, BUILTIN, BackendError, Caller, Dependency, Error, EventKind, Invalid, Kind,
    MAX_SUBSCRIBERS, Notice, Property, PropertyId, PropertySet, ReadOnly, Registry, ResetReason,
    Schema, SchemaError, Scope, Store, Visibility, Word, key,
};
use mock::{Client, Shared};

fn id(registry: &Registry, key: &str) -> PropertyId {
    registry.find(key).unwrap()
}
fn events(registry: &mut Registry) -> Vec<String> {
    let mut lines = Vec::new();
    registry.drain_events(|event| lines.push(event.to_string()));
    lines
}
fn commit(registry: &mut Registry, client: &mut Client, key: &str, value: u32) {
    let proposal = registry.propose_key(key, value).unwrap();
    registry.commit(client, proposal).unwrap();
}
const CONTROLS: Store = Store::Setting(Setting::DesktopControls);

#[test]
fn builtin_schema_is_valid_and_covers_every_persisted_bit() {
    assert_eq!(validate(&BUILTIN), Ok(()));
    assert_eq!(BUILTIN.words.len(), 9);
    assert_eq!(BUILTIN.properties.len(), 36);
    // Words are storage Setting ids 0..=8 in order, all per user.
    for (index, word) in BUILTIN.words.iter().enumerate() {
        match word.store {
            Store::Setting(setting) => assert_eq!(setting as usize, index),
            Store::Registry { .. } => panic!("no typed registry value is persisted today"),
        }
        assert_eq!((word.scope, word.write), (Scope::User, Authority::User));
    }
    let covered = |word: u8| {
        BUILTIN
            .properties
            .iter()
            .filter(|p| p.word == word)
            .fold(0u32, |mask, p| mask | p.mask())
    };
    assert_eq!(covered(3), u32::MAX, "every DesktopControls bit");
    assert_eq!(covered(2), 0b11_1111, "dark bit and the 18 backgrounds");
    assert_eq!(covered(4), (1 << 15) - 1, "ShellLayout flags");
    assert_eq!(covered(5), (1 << 20) - 1, "ShellOrder ranks");
    assert_eq!(covered(6), (1 << 28) - 1, "InputMethods");
    for word in [0, 1, 7, 8] {
        assert_eq!(covered(word), u32::MAX);
    }
    // Stable keys resolve.
    let registry = Registry::new(Caller::User);
    for key in [
        key::TASKBAR_POSITION,
        key::REDUCE_MOTION,
        key::TIME_ZONE,
        key::REGIONAL_FORMAT,
        key::INPUT_METHODS,
        key::DISPLAY_SCALE,
        key::START_PIN_ORDER,
    ] {
        assert!(registry.find(key).is_some(), "{key}");
    }
}

#[test]
fn defaults_and_bit_encodings_match_the_persisted_formats() {
    let registry = Registry::new(Caller::User);
    let get = |key: &str| registry.get_key(key).unwrap();
    assert_eq!(get(key::CONTEXT_INLINE_ACTIONS), 1);
    assert_eq!(get(key::CONTEXT_PROPERTIES_PLACEMENT), 1);
    assert_eq!(get(key::TIME_ZONE), u32::from(door_locale::tz::DEFAULT_ID));
    assert_eq!(get(key::REGIONAL_FORMAT), 1);
    assert_eq!(get(key::INPUT_METHODS), 1);
    assert_eq!(get(key::DISPLAY_SCALE), 0);
    // Each flag property maps onto exactly its Controls constant.
    for (key, bit) in [
        (key::DESKTOP_ICONS, Controls::SHOW_DESKTOP_ICONS),
        (key::REDUCE_TRANSPARENCY, Controls::REDUCE_TRANSPARENCY),
        (key::CONTEXT_CLASSIC, Controls::CLASSIC_CONTEXT_MENU),
        (key::TASKBAR_SEARCH, Controls::SEARCH_ICON_ONLY),
        (key::REDUCE_MOTION, Controls::REDUCE_MOTION),
        (key::START_HIDE_PINNED, Controls::START_HIDE_PINNED),
        (key::START_HIDE_RECENT, Controls::START_HIDE_RECENT),
        (key::START_HIDE_ALL, Controls::START_HIDE_ALL),
        (key::START_HIDE_ACCOUNT, Controls::START_HIDE_ACCOUNT),
        (
            key::START_HIDE_RECENT_FILES,
            Controls::START_HIDE_RECENT_FILES,
        ),
        (
            key::CONTEXT_HIDE_FILE_ACTIONS,
            Controls::CONTEXT_HIDE_FILE_ACTIONS,
        ),
        (
            key::CONTEXT_HIDE_MORE_OPTIONS,
            Controls::CONTEXT_HIDE_MORE_OPTIONS,
        ),
        (key::CONTRAST, Controls::CONTRAST_ON),
        (key::TASKBAR_ALIGNMENT, 4),
        (key::TASKBAR_SIZE, 8),
    ] {
        let p = registry.property(id(&registry, key));
        assert_eq!(p.mask(), bit, "{key}");
    }
    // Committed values land in the persisted word unchanged.
    let shared = Shared::new(&BUILTIN);
    let mut client = Client::new(&shared, false);
    let mut registry = Registry::new(Caller::User);
    registry.load(&mut client).unwrap();
    commit(&mut registry, &mut client, key::TEXT_SIZE, 25);
    commit(&mut registry, &mut client, key::CONTRAST_THEME, 2);
    commit(&mut registry, &mut client, key::CONTRAST, 1);
    commit(&mut registry, &mut client, key::START_SIZE, 2);
    commit(&mut registry, &mut client, key::TASKBAR_POSITION, 2);
    let controls = Controls::from_bits(shared.borrow().raw(CONTROLS).unwrap()).unwrap();
    assert_eq!(controls.text_percent(), 125);
    assert_eq!(controls.contrast(), Some(ContrastTheme::Desert));
    assert_eq!(controls.start_size(), 2);
    assert_eq!(controls.edge(), door_service_protocol::desktop::Edge::Left);
    assert!(controls.inline_actions() && controls.properties_bottom());
    commit(&mut registry, &mut client, key::START_ALL_VIEW, 2);
    commit(&mut registry, &mut client, key::START_ALL_SORTED, 1);
    let layout = shared
        .borrow()
        .raw(Store::Setting(Setting::ShellLayout))
        .unwrap();
    let layout = ShellLayout::from_parts(layout, 0).unwrap();
    assert_eq!((layout.view(), layout.sorted()), (2, true));
    commit(&mut registry, &mut client, key::DISPLAY_SCALE, 125);
    assert_eq!(
        shared.borrow().raw(Store::Setting(Setting::ScalePercent)),
        Some(125)
    );
}

#[test]
fn validation_rejects_wrong_types_ranges_and_combinations() {
    let registry = Registry::new(Caller::User);
    let p = |key: &str| id(&registry, key);
    let invalid = |e| Err(Error::Invalid(e));
    assert_eq!(
        registry.validate(p(key::REDUCE_MOTION), 2),
        invalid(Invalid::Type)
    );
    assert_eq!(
        registry.validate(p(key::DISPLAY_SCALE), 110),
        invalid(Invalid::Type)
    );
    assert_eq!(
        registry.validate(p(key::DISPLAY_SCALE), 325),
        invalid(Invalid::Type)
    );
    assert_eq!(registry.validate(p(key::DISPLAY_SCALE), 0), Ok(()));
    assert_eq!(registry.validate(p(key::DISPLAY_SCALE), 175), Ok(()));
    assert_eq!(
        registry.validate(p(key::START_LAYOUT), 3),
        invalid(Invalid::Type)
    );
    assert_eq!(
        registry.validate(p(key::TEXT_SIZE), 126),
        invalid(Invalid::Type)
    );
    assert_eq!(
        registry.validate(p(key::TIME_ZONE), 9999),
        invalid(Invalid::Type)
    );
    assert_eq!(
        registry.validate(p(key::TASKBAR_UNPINNED), 64),
        invalid(Invalid::Type)
    );
    assert_eq!(registry.validate(p(key::TASKBAR_UNPINNED), 63), Ok(()));
    // The input method list is validated as a whole word.
    assert_eq!(
        registry.validate(p(key::INPUT_METHODS), 0),
        invalid(Invalid::Combination)
    );
    assert_eq!(
        registry.validate(p(key::INPUT_METHODS), 1 << 27),
        invalid(Invalid::Type),
        "wider than the 27-bit field"
    );
    assert_eq!(registry.label(p(key::DISPLAY_SCALE), 0), Some("Automatic"));
    assert_eq!(registry.label(p(key::TASKBAR_POSITION), 3), Some("Right"));
    assert_eq!(
        registry.label(p(key::TIME_ZONE), 16),
        Some("(UTC) Coordinated Universal Time")
    );
}

#[test]
fn unknown_keys_and_invalid_schemas_are_refused() {
    let registry = Registry::new(Caller::User);
    assert_eq!(registry.get_key("taskbar.nope"), Err(Error::UnknownKey));
    assert_eq!(
        registry.propose_key("taskbar.nope", 1).map(|_| ()),
        Err(Error::UnknownKey)
    );
    fn leak(words: Vec<Word>, properties: Vec<Property>) -> &'static Schema {
        Box::leak(Box::new(Schema {
            words: Box::leak(words.into_boxed_slice()),
            properties: Box::leak(properties.into_boxed_slice()),
        }))
    }
    let word = BUILTIN.words[3];
    let base = BUILTIN.properties[id(&registry, key::REDUCE_MOTION).index()];
    let base = Property { word: 0, ..base };
    let check = |properties: Vec<Property>| validate(leak(vec![word], properties));
    assert_eq!(check(vec![base]), Ok(()));
    assert_eq!(
        check(vec![base, base]),
        Err(SchemaError::DuplicateKey(key::REDUCE_MOTION))
    );
    assert_eq!(
        check(vec![
            base,
            Property {
                key: "other",
                ..base
            }
        ]),
        Err(SchemaError::Overlap("other"))
    );
    assert_eq!(
        check(vec![Property {
            key: "Bad Key",
            ..base
        }]),
        Err(SchemaError::KeySyntax("Bad Key"))
    );
    assert_eq!(
        check(vec![Property { default: 1, ..base }]),
        Err(SchemaError::Default(key::REDUCE_MOTION))
    );
    assert_eq!(
        check(vec![Property {
            shift: 31,
            width: 2,
            ..base
        }]),
        Err(SchemaError::Field(key::REDUCE_MOTION))
    );
    assert_eq!(
        check(vec![Property { word: 1, ..base }]),
        Err(SchemaError::Field(key::REDUCE_MOTION))
    );
    assert_eq!(
        check(vec![Property {
            depends: &[Dependency::ActiveWhen {
                key: "missing",
                value: 1
            }],
            ..base
        }]),
        Err(SchemaError::Dependency(key::REDUCE_MOTION))
    );
    assert_eq!(
        check(vec![Property {
            anchor: Some("not-a-section"),
            ..base
        }]),
        Err(SchemaError::Anchor(key::REDUCE_MOTION))
    );
    assert_eq!(
        check(vec![Property {
            kind: Kind::Choice(&[]),
            ..base
        }]),
        Err(SchemaError::Kind(key::REDUCE_MOTION))
    );
    assert_eq!(
        validate(leak(
            vec![Word {
                default: 3 << 7,
                ..word
            }],
            vec![]
        )),
        Err(SchemaError::WordDefault(0))
    );
    let schema = leak(vec![word], vec![base, base]);
    assert!(Registry::with_schema(schema, Caller::User).is_err());
}

#[test]
fn a_commit_propagates_to_another_process_through_the_change_sequence() {
    let shared = Shared::new(&BUILTIN);
    let (mut a_client, mut b_client) = (Client::new(&shared, false), Client::new(&shared, false));
    let (mut a, mut b) = (Registry::new(Caller::User), Registry::new(Caller::User));
    a.load(&mut a_client).unwrap();
    b.load(&mut b_client).unwrap();
    assert!(b.poll(&mut b_client).unwrap().is_empty());
    let watcher = b.subscribe(PropertySet::ALL).unwrap();
    commit(&mut a, &mut a_client, key::TASKBAR_POSITION, 1);
    assert_eq!(
        events(&mut a),
        ["DOOR_P14 SETTINGS commit key=taskbar.position value=1"]
    );
    let changed = b.poll(&mut b_client).unwrap();
    let position = id(&b, key::TASKBAR_POSITION);
    let search = id(&b, key::TASKBAR_SEARCH);
    assert!(changed.contains(position));
    // The search box is forced to an icon off the bottom edge.
    assert!(changed.contains(search));
    assert_eq!(
        (b.get(position), b.get(search), b.effective(search)),
        (1, 0, 1)
    );
    assert_eq!(
        events(&mut b),
        ["DOOR_P14 SETTINGS observed key=taskbar.position value=1"]
    );
    assert_eq!(b.take_changes(watcher), changed);
    assert!(b.take_changes(watcher).is_empty());
    // Nothing new: no re-read noise; the writer sees its own change once.
    assert!(b.poll(&mut b_client).unwrap().is_empty());
    assert!(a.poll(&mut a_client).unwrap().is_empty());
    assert!(events(&mut a).is_empty());
    // And back the other way.
    commit(&mut b, &mut b_client, key::REDUCE_MOTION, 1);
    assert!(
        a.poll(&mut a_client)
            .unwrap()
            .contains(id(&a, key::REDUCE_MOTION))
    );
    assert_eq!(
        events(&mut a),
        ["DOOR_P14 SETTINGS observed key=accessibility.reduce_motion value=1"]
    );
}

#[test]
fn a_concurrent_change_to_another_field_is_kept_and_to_the_same_one_is_stale() {
    let shared = Shared::new(&BUILTIN);
    let (mut a_client, mut b_client) = (Client::new(&shared, false), Client::new(&shared, false));
    let (mut a, mut b) = (Registry::new(Caller::User), Registry::new(Caller::User));
    a.load(&mut a_client).unwrap();
    b.load(&mut b_client).unwrap();
    let proposal = a.propose_key(key::TASKBAR_POSITION, 1).unwrap();
    commit(&mut b, &mut b_client, key::TASKBAR_SIZE, 1);
    a.commit(&mut a_client, proposal).unwrap();
    let controls = Controls::from_bits(shared.borrow().raw(CONTROLS).unwrap()).unwrap();
    assert!(controls.small());
    assert_eq!(controls.edge(), door_service_protocol::desktop::Edge::Top);
    let proposal = a.propose_key(key::TASKBAR_POSITION, 2).unwrap();
    // B has not observed A's position yet: its own proposal is stale too.
    let stale = b.propose_key(key::TASKBAR_POSITION, 3).unwrap();
    assert_eq!(b.commit(&mut b_client, stale), Err(Error::Stale));
    assert_eq!(
        b.get_key(key::TASKBAR_POSITION),
        Ok(1),
        "read back meanwhile"
    );
    commit(&mut b, &mut b_client, key::TASKBAR_POSITION, 3);
    assert_eq!(a.commit(&mut a_client, proposal), Err(Error::Stale));
    assert_eq!(a.get_key(key::TASKBAR_POSITION), Ok(3));
    assert_eq!(shared.borrow().raw(CONTROLS).unwrap() & 3, 3);
}

#[test]
fn a_restarted_backend_is_resubscribed_and_reread() {
    let shared = Shared::new(&BUILTIN);
    let (mut a_client, mut b_client) = (Client::new(&shared, false), Client::new(&shared, false));
    let (mut a, mut b) = (Registry::new(Caller::User), Registry::new(Caller::User));
    a.load(&mut a_client).unwrap();
    b.load(&mut b_client).unwrap();
    shared.borrow_mut().restart();
    // Old connections fail; nothing changes locally.
    assert_eq!(
        b.poll(&mut b_client),
        Err(Error::Backend(BackendError::Disconnected))
    );
    let proposal = a.propose_key(key::TIME_ZONE, 1).unwrap();
    assert_eq!(
        a.commit(&mut a_client, proposal),
        Err(Error::Backend(BackendError::Disconnected))
    );
    assert_eq!(a.get_key(key::TIME_ZONE), Ok(16));
    // A reconnects and writes while B is still disconnected.
    a_client.reconnect();
    a.disconnected();
    a.commit(&mut a_client, proposal).unwrap();
    b_client.reconnect();
    let changed = b.poll(&mut b_client).unwrap();
    assert!(changed.contains(id(&b, key::TIME_ZONE)));
    assert_eq!(b.get_key(key::TIME_ZONE), Ok(1));
    assert!(
        events(&mut b).contains(&"DOOR_P14 SETTINGS observed key=time.zone value=1".to_string())
    );
    // The renewed subscription keeps working.
    commit(&mut a, &mut a_client, key::REGIONAL_FORMAT, 2);
    assert!(
        b.poll(&mut b_client)
            .unwrap()
            .contains(id(&b, key::REGIONAL_FORMAT))
    );
}

#[test]
fn an_interrupted_write_keeps_the_old_value() {
    let shared = Shared::new(&BUILTIN);
    let mut client = Client::new(&shared, false);
    let mut registry = Registry::new(Caller::User);
    registry.load(&mut client).unwrap();
    let watcher = registry.subscribe(PropertySet::ALL).unwrap();
    shared.borrow_mut().fail_writes = 1;
    let proposal = registry.propose_key(key::DESKTOP_ICONS, 1).unwrap();
    assert_eq!(
        registry.commit(&mut client, proposal),
        Err(Error::Backend(BackendError::Io))
    );
    assert_eq!(registry.get_key(key::DESKTOP_ICONS), Ok(0));
    assert_eq!(shared.borrow().raw(CONTROLS), None);
    assert!(registry.take_changes(watcher).is_empty());
    assert!(events(&mut registry).is_empty());
    let committed = registry.commit(&mut client, proposal).unwrap();
    assert_eq!(committed.apply, Apply::Live);
    assert_eq!(registry.get_key(key::DESKTOP_ICONS), Ok(1));
    assert_eq!(
        shared.borrow().raw(CONTROLS),
        Some(Controls::DEFAULT.bits() | Controls::SHOW_DESKTOP_ICONS)
    );
}

#[test]
fn corrupt_values_read_as_defaults_with_a_notice_and_are_rewritten() {
    let shared = Shared::new(&BUILTIN);
    // Start layout 3 is invalid; storage refuses to return the word.
    shared
        .borrow_mut()
        .poke(CONTROLS, (3 << 7) | Controls::REDUCE_MOTION);
    let mut client = Client::new(&shared, false);
    let mut registry = Registry::new(Caller::User);
    registry.load(&mut client).unwrap();
    assert_eq!(registry.get_key(key::START_LAYOUT), Ok(0));
    assert_eq!(registry.get_key(key::REDUCE_MOTION), Ok(0));
    let notice = registry.notice().unwrap();
    assert_eq!(notice.message(), Notice::MESSAGE);
    assert_eq!(
        Notice::MESSAGE,
        "Some settings were reset to their defaults because they were damaged"
    );
    assert_eq!(notice.reset.len(), 22, "every DesktopControls property");
    assert_eq!(notice.rewritten, notice.reset);
    assert!(!notice.storage_corrupt);
    assert_eq!(
        shared.borrow().raw(CONTROLS),
        Some(Controls::DEFAULT.bits())
    );
    let lines = events(&mut registry);
    assert!(lines.contains(&"DOOR_P14 SETTINGS reset key=start.layout reason=corrupt".into()));
    assert_eq!(lines.len(), 22);
    registry.dismiss_notice();
    assert_eq!(registry.notice(), None);
    // Repaired: later reads are clean and changes commit normally.
    assert!(registry.refresh(&mut client).unwrap().is_empty());
    assert_eq!(registry.notice(), None);
    commit(&mut registry, &mut client, key::START_LAYOUT, 1);
    assert_eq!(registry.get_key(key::START_LAYOUT), Ok(1));
}

#[test]
fn a_corrupt_value_observed_later_is_reset_once_and_others_see_the_default() {
    let shared = Shared::new(&BUILTIN);
    let (mut a_client, mut b_client) = (Client::new(&shared, false), Client::new(&shared, false));
    let (mut a, mut b) = (Registry::new(Caller::User), Registry::new(Caller::User));
    a.load(&mut a_client).unwrap();
    b.load(&mut b_client).unwrap();
    commit(&mut a, &mut a_client, key::TIME_ZONE, 1);
    b.poll(&mut b_client).unwrap();
    events(&mut b);
    shared
        .borrow_mut()
        .poke(Store::Setting(Setting::TimeZone), 9999);
    b.poll(&mut b_client).unwrap();
    assert_eq!(b.get_key(key::TIME_ZONE), Ok(16));
    assert_eq!(
        events(&mut b),
        [
            "DOOR_P14 SETTINGS reset key=time.zone reason=corrupt",
            "DOOR_P14 SETTINGS observed key=time.zone value=16"
        ]
    );
    a.poll(&mut a_client).unwrap();
    assert_eq!(a.get_key(key::TIME_ZONE), Ok(16));
    assert_eq!(a.notice(), None, "A read the repaired value");
}

#[test]
fn storage_corruption_gives_defaults_without_writing() {
    let shared = Shared::new(&BUILTIN);
    let zone = Store::Setting(Setting::TimeZone);
    shared.borrow_mut().poke(zone, 1);
    shared.borrow_mut().corrupt.insert(mock::slot(zone));
    let mut client = Client::new(&shared, false);
    let mut registry = Registry::new(Caller::User);
    registry.load(&mut client).unwrap();
    let time_zone = id(&registry, key::TIME_ZONE);
    assert_eq!(registry.get(time_zone), 16);
    let notice = registry.notice().unwrap();
    assert!(notice.storage_corrupt);
    assert!(notice.reset.contains(time_zone));
    assert!(notice.rewritten.is_empty());
    assert!(shared.borrow().writes.is_empty());
    assert_eq!(
        events(&mut registry),
        ["DOOR_P14 SETTINGS reset key=time.zone reason=storage-corrupt"]
    );
    assert_eq!(
        registry.read_only(time_zone),
        Some(ReadOnly::StorageCorrupt)
    );
    assert_eq!(
        registry.propose(time_zone, 1).map(|_| ()),
        Err(Error::ReadOnly(ReadOnly::StorageCorrupt))
    );
    let hit = registry.search("time zone", Caller::User).next().unwrap();
    assert!(hit.read_only);
    // Other words still work; the damaged one is not reported again.
    commit(&mut registry, &mut client, key::REGIONAL_FORMAT, 2);
    registry.refresh(&mut client).unwrap();
    assert!(!events(&mut registry).iter().any(|l| l.contains("reset")));
    assert_eq!(shared.borrow().writes.len(), 1);
    // After repair the stored value is read again.
    shared.borrow_mut().corrupt.clear();
    registry.refresh(&mut client).unwrap();
    assert_eq!(registry.get(time_zone), 1);
    assert_eq!(registry.read_only(time_zone), None);
}

// A schema with user and machine registry words.
const USER: Store = Store::Registry {
    root: RegistryRoot::CurrentUser,
    path: "Software\\Door\\Test",
    name: "Value",
};
const MACHINE: Store = Store::Registry {
    root: RegistryRoot::LocalMachine,
    path: "SOFTWARE\\Door\\Test",
    name: "Policy",
};
const fn test_property(
    key: &'static str,
    title: &'static str,
    word: u8,
    shift: u8,
    width: u8,
    kind: Kind,
    apply: Apply,
) -> Property {
    Property {
        key,
        title,
        keywords: "test",
        kind,
        default: 0,
        word,
        shift,
        width,
        apply,
        page: Page::SystemAdvanced,
        anchor: None,
        visibility: Visibility::Public,
        depends: &[],
    }
}
static TEST: Schema = Schema {
    words: &[
        Word {
            store: USER,
            default: 0,
            valid: |v| v < 8,
            scope: Scope::User,
            write: Authority::User,
            couple: None,
        },
        Word {
            store: MACHINE,
            default: 0,
            valid: |v| v < 8,
            scope: Scope::Machine,
            write: Authority::Administrator,
            couple: None,
        },
    ],
    properties: &[
        test_property("user.flag", "User flag", 0, 0, 1, Kind::Bool, Apply::Live),
        Property {
            depends: &[Dependency::ActiveWhen {
                key: "user.flag",
                value: 1,
            }],
            ..test_property(
                "user.mode",
                "User mode",
                0,
                1,
                2,
                Kind::Choice(&[(0, "A"), (1, "B"), (2, "C")]),
                Apply::SignOut,
            )
        },
        test_property(
            "machine.policy",
            "Machine policy",
            1,
            0,
            1,
            Kind::Bool,
            Apply::Restart,
        ),
        Property {
            visibility: Visibility::Private,
            ..test_property(
                "machine.secret",
                "Machine secret level",
                1,
                1,
                2,
                Kind::Choice(&[(0, "Low"), (1, "High")]),
                Apply::Live,
            )
        },
    ],
};

#[test]
fn machine_settings_are_read_only_for_users_and_private_ones_are_hidden() {
    let shared = Shared::new(&TEST);
    shared.borrow_mut().poke(MACHINE, 1);
    let mut client = Client::new(&shared, false);
    let mut user = Registry::with_schema(&TEST, Caller::User).unwrap();
    user.load(&mut client).unwrap();
    let policy = id(&user, "machine.policy");
    assert_eq!(user.get(policy), 1, "machine values are readable");
    assert_eq!(user.read_only(policy), Some(ReadOnly::Permission));
    assert_eq!(
        user.propose(policy, 0).map(|_| ()),
        Err(Error::ReadOnly(ReadOnly::Permission))
    );
    let hits: Vec<_> = user.search("machine", Caller::User).collect();
    assert_eq!(hits.len(), 1, "the private one is omitted");
    assert_eq!((hits[0].key, hits[0].read_only), ("machine.policy", true));
    let hits: Vec<_> = user.search("machine", Caller::Administrator).collect();
    assert_eq!(hits.len(), 2);
    assert!(hits.iter().all(|hit| !hit.read_only));
    // An administrator registry over a connection without system
    // authority: storage refuses and the old value stays.
    let mut admin = Registry::with_schema(&TEST, Caller::Administrator).unwrap();
    admin.load(&mut client).unwrap();
    let proposal = admin.propose(policy, 0).unwrap();
    assert_eq!(
        admin.commit(&mut client, proposal),
        Err(Error::Backend(BackendError::Denied))
    );
    assert_eq!(admin.get(policy), 1);
    // With system authority it commits; the user registry observes it.
    let mut system = Client::new(&shared, true);
    let committed = admin.commit(&mut system, proposal).unwrap();
    assert_eq!(committed.apply, Apply::Restart);
    assert!(user.poll(&mut client).unwrap().contains(policy));
    assert_eq!(user.get(policy), 0);
    // A damaged machine value: defaults, but a user cannot rewrite it.
    shared.borrow_mut().poke(MACHINE, 7);
    user.poll(&mut client).unwrap();
    let notice = user.notice().unwrap();
    assert!(notice.reset.contains(policy) && notice.rewritten.is_empty());
    assert_eq!(shared.borrow().raw(MACHINE), Some(7));
    // Values the caller cannot read never appear.
    shared.borrow_mut().unreadable.insert(mock::slot(MACHINE));
    user.refresh(&mut client).unwrap();
    assert_eq!(user.search("machine", Caller::User).count(), 0);
    assert_eq!(user.read_only(policy), Some(ReadOnly::Unreadable));
}

#[test]
fn proposals_report_apply_timing_and_dependency_effects() {
    let shared = Shared::new(&TEST);
    let mut client = Client::new(&shared, false);
    let mut registry = Registry::with_schema(&TEST, Caller::User).unwrap();
    registry.load(&mut client).unwrap();
    let (flag, mode) = (id(&registry, "user.flag"), id(&registry, "user.mode"));
    assert_eq!(registry.state(mode).inactive_by, Some(flag));
    let proposal = registry.propose(mode, 2).unwrap();
    assert_eq!(proposal.apply, Apply::SignOut);
    assert_eq!(proposal.inactive_by, Some(flag));
    assert!(proposal.effects.is_empty());
    assert_eq!(
        registry.commit(&mut client, proposal).unwrap().apply,
        Apply::SignOut
    );
    // Turning the flag on activates the mode: an effect of the change.
    let proposal = registry.propose(flag, 1).unwrap();
    assert!(proposal.effects.contains(mode));
    assert_eq!(proposal.apply, Apply::SignOut, "the latest affected timing");
    let committed = registry.commit(&mut client, proposal).unwrap();
    assert!(committed.changed.contains(flag) && committed.changed.contains(mode));
    assert_eq!(registry.state(mode).inactive_by, None);
    // Re-committing the same value writes nothing.
    let writes = shared.borrow().writes.len();
    let proposal = registry.propose(flag, 1).unwrap();
    assert!(proposal.unchanged());
    assert!(
        registry
            .commit(&mut client, proposal)
            .unwrap()
            .changed
            .is_empty()
    );
    assert_eq!(shared.borrow().writes.len(), writes);
}

#[test]
fn builtin_dependencies_and_shared_storage_are_reported() {
    let shared = Shared::new(&BUILTIN);
    let mut client = Client::new(&shared, false);
    let mut registry = Registry::new(Caller::User);
    registry.load(&mut client).unwrap();
    let p = |key: &str| registry.find(key).unwrap();
    let (search, contrast, theme) = (
        p(key::TASKBAR_SEARCH),
        p(key::CONTRAST),
        p(key::CONTRAST_THEME),
    );
    let (mode, console) = (p(key::APPEARANCE_MODE), p(key::CONSOLE_THEME));
    let recent_files = p(key::START_HIDE_RECENT_FILES);
    assert!(
        registry
            .propose_key(key::TASKBAR_POSITION, 1)
            .unwrap()
            .effects
            .contains(search)
    );
    assert_eq!(registry.state(theme).inactive_by, Some(contrast));
    assert!(
        registry
            .propose(contrast, 1)
            .unwrap()
            .effects
            .contains(theme)
    );
    assert!(
        registry
            .propose_key(key::START_HIDE_RECENT, 1)
            .unwrap()
            .effects
            .contains(recent_files)
    );
    // Dark mode and the console theme share storage-core's dark bit.
    let proposal = registry.propose(mode, 1).unwrap();
    assert_eq!(proposal.effects, PropertySet::of(console));
    let committed = registry.commit(&mut client, proposal).unwrap();
    assert!(committed.changed.contains(console));
    assert_eq!(registry.get(console), 1);
    assert_eq!(
        events(&mut registry),
        [
            "DOOR_P14 SETTINGS commit key=appearance.mode value=1",
            "DOOR_P14 SETTINGS commit key=console.theme value=1",
        ]
    );
    assert_eq!(shared.borrow().raw(Store::Setting(Setting::Theme)), Some(1));
    commit(&mut registry, &mut client, key::CONSOLE_THEME, 0);
    assert_eq!(registry.get(mode), 0);
    assert!(
        registry.refresh(&mut client).unwrap().is_empty(),
        "prediction matches storage"
    );
}

#[test]
fn subscriptions_filter_and_have_fixed_capacity() {
    let shared = Shared::new(&BUILTIN);
    let mut client = Client::new(&shared, false);
    let mut registry = Registry::new(Caller::User);
    registry.load(&mut client).unwrap();
    let motion = id(&registry, key::REDUCE_MOTION);
    let only_motion = registry.subscribe(PropertySet::of(motion)).unwrap();
    let mut others = Vec::new();
    for _ in 1..MAX_SUBSCRIBERS {
        others.push(registry.subscribe(PropertySet::ALL).unwrap());
    }
    assert_eq!(registry.subscribe(PropertySet::ALL), Err(Error::Capacity));
    commit(&mut registry, &mut client, key::DESKTOP_ICONS, 1);
    assert!(registry.take_changes(only_motion).is_empty());
    commit(&mut registry, &mut client, key::REDUCE_MOTION, 1);
    assert_eq!(registry.take_changes(only_motion), PropertySet::of(motion));
    assert_eq!(registry.take_changes(others[0]).len(), 2);
    registry.unsubscribe(others[0]);
    assert!(registry.take_changes(others[0]).is_empty());
    let renewed = registry.subscribe(PropertySet::ALL).unwrap();
    assert_ne!(renewed, others[0], "a stale handle never aliases a new one");
}

#[test]
fn the_event_queue_drops_the_oldest_when_full() {
    let shared = Shared::new(&BUILTIN);
    let mut client = Client::new(&shared, false);
    let mut registry = Registry::new(Caller::User);
    registry.load(&mut client).unwrap();
    for i in 0..130 {
        commit(&mut registry, &mut client, key::REDUCE_MOTION, (i + 1) % 2);
    }
    assert_eq!(registry.events_lost(), 2);
    let lines = events(&mut registry);
    assert_eq!(lines.len(), 128);
    assert_eq!(
        lines.last().unwrap(),
        "DOOR_P14 SETTINGS commit key=accessibility.reduce_motion value=0"
    );
}

#[test]
fn search_matches_titles_and_keywords_with_page_and_section() {
    let registry = Registry::new(Caller::User);
    let hits: Vec<_> = registry.search("Transparency", Caller::User).collect();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].key, key::REDUCE_TRANSPARENCY);
    assert_eq!(
        (hits[0].page, hits[0].anchor),
        (Page::Colors, Some("transparency"))
    );
    assert!(!hits[0].read_only);
    let zone: Vec<_> = registry.search("time zone", Caller::User).collect();
    assert_eq!(zone[0].anchor, Some("time-zone"));
    assert_eq!(registry.search("", Caller::User).count(), 0);
    assert_eq!(
        registry.search("pin order", Caller::User).count(),
        0,
        "internal"
    );
    assert!(
        registry
            .search("dark mode", Caller::User)
            .any(|hit| hit.key == key::APPEARANCE_MODE)
    );
    assert_eq!(registry.on_page(Page::ContrastThemes).count(), 2);
    let _ = (EventKind::Reset(ResetReason::Corrupt), Caller::User);
}

#[test]
fn each_property_reports_its_location_scope_and_authority() {
    let registry = Registry::new(Caller::User);
    let word = registry.word(id(&registry, key::TIME_ZONE));
    assert_eq!(word.store, Store::Setting(Setting::TimeZone));
    assert_eq!((word.scope, word.write), (Scope::User, Authority::User));
    let machine = Registry::with_schema(&TEST, Caller::User).unwrap();
    let word = machine.word(id(&machine, "machine.policy"));
    assert_eq!((word.store, word.scope), (MACHINE, Scope::Machine));
    assert_eq!(word.write, Authority::Administrator);
}
