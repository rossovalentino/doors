//! Task definitions as COM objects: ITaskDefinition and the parts it hands
//! out (IRegistrationInfo, ITriggerCollection and every trigger,
//! IRepetitionPattern, IActionCollection and every action, ITaskSettings
//! with its idle, network and maintenance settings, IPrincipal, the named
//! value collections). A definition is one `Def`; its triggers and actions
//! are items with one COM object each at a time (the collection's Item
//! hands back the same object, as Windows does). Properties are read and
//! written through field numbers (`f`), one generic accessor per type.
use super::*;

/// A trigger or action: its value and the COM object showing it (0: none).
pub(crate) type Item<T> = Lock<(T, u64)>;
pub(crate) struct Parts {
    info: Registration,
    triggers: Vec<Arc<Item<Trigger>>>,
    settings: Settings,
    principal: Principal,
    actions: Vec<Arc<Item<Action>>>,
    context: Option<String>,
    data: Option<String>,
    /// ITaskSettings3's maintenance settings (not in the XML: deviation).
    maintenance: Option<(Option<String>, Option<String>, bool)>,
    /// IPrincipal2 (not in the XML: deviation).
    sid_type: i32,
    privileges: Vec<String>,
}
pub(crate) struct Def {
    parts: Lock<Parts>,
}
pub(crate) enum Part {
    Definition(Arc<Def>),
    Info(Arc<Def>),
    Triggers(Arc<Def>),
    Trigger(Arc<Item<Trigger>>),
    Repetition(Arc<Item<Trigger>>),
    Actions(Arc<Def>),
    Action(Arc<Item<Action>>),
    Settings(Arc<Def>),
    Idle(Arc<Def>),
    Network(Arc<Def>),
    Maintenance(Arc<Def>),
    Principal(Arc<Def>),
    TriggerPairs(Arc<Item<Trigger>>),
    ActionPairs(Arc<Item<Action>>),
    Pair(Arc<Lock<(String, String)>>),
}
fn part(this: u64) -> Option<&'static Part> {
    match &obj(this).kind {
        Kind::Def(p) => Some(p),
        _ => None,
    }
}
fn def_of(this: u64) -> Option<&'static Arc<Def>> {
    match part(this)? {
        Part::Definition(d)
        | Part::Info(d)
        | Part::Triggers(d)
        | Part::Actions(d)
        | Part::Settings(d)
        | Part::Idle(d)
        | Part::Network(d)
        | Part::Maintenance(d)
        | Part::Principal(d) => Some(d),
        _ => None,
    }
}
fn new_def(d: Definition) -> Arc<Def> {
    let def = Arc::new(Def {
        parts: Lock::new(Parts {
            info: Registration::default(),
            triggers: Vec::new(),
            settings: Definition::from_defaults().settings,
            principal: Definition::from_defaults().principal,
            actions: Vec::new(),
            context: None,
            data: None,
            maintenance: None,
            sid_type: 2,
            privileges: Vec::new(),
        }),
    });
    load(&def, d);
    def
}
fn load(def: &Def, d: Definition) {
    def.parts.with(|p| {
        p.info = d.info;
        p.triggers = d
            .triggers
            .into_iter()
            .map(|t| Arc::new(Lock::new((t, 0))))
            .collect();
        p.settings = d.settings;
        p.principal = d.principal;
        p.actions = d
            .actions
            .into_iter()
            .map(|a| Arc::new(Lock::new((a, 0))))
            .collect();
        p.context = d.context;
        p.data = d.data;
    });
}
fn snapshot(def: &Def) -> Definition {
    def.parts.with(|p| Definition {
        info: p.info.clone(),
        triggers: p
            .triggers
            .iter()
            .map(|t| t.with(|(t, _)| t.clone()))
            .collect(),
        settings: p.settings.clone(),
        principal: p.principal.clone(),
        actions: p
            .actions
            .iter()
            .map(|a| a.with(|(a, _)| a.clone()))
            .collect(),
        context: p.context.clone(),
        data: p.data.clone(),
    })
}
pub(crate) fn new_definition(d: Definition) -> u64 {
    make(Part::Definition(new_def(d)))
}
/// The definition behind one of our ITaskDefinition pointers.
pub(crate) fn definition_of(p: u64) -> Option<Definition> {
    if !ours(p) {
        return None;
    }
    match part(p)? {
        Part::Definition(d) => Some(snapshot(d)),
        _ => None,
    }
}

// ---- object identity of triggers and actions ---------------------------------------------
fn trigger_table(kind: u32) -> (*const (), Guid) {
    let t = |table: *const [*const ()]| table as *const ();
    match kind {
        trigger::IDLE => (t(&IDLE_TRIGGER.0), iid::IDLE_TRIGGER),
        trigger::LOGON => (t(&LOGON_TRIGGER.0), iid::LOGON_TRIGGER),
        trigger::SESSION_STATE_CHANGE => (t(&SESSION_TRIGGER.0), iid::SESSION_TRIGGER),
        trigger::EVENT => (t(&EVENT_TRIGGER.0), iid::EVENT_TRIGGER),
        trigger::TIME => (t(&TIME_TRIGGER.0), iid::TIME_TRIGGER),
        trigger::DAILY => (t(&DAILY_TRIGGER.0), iid::DAILY_TRIGGER),
        trigger::WEEKLY => (t(&WEEKLY_TRIGGER.0), iid::WEEKLY_TRIGGER),
        trigger::MONTHLY => (t(&MONTHLY_TRIGGER.0), iid::MONTHLY_TRIGGER),
        trigger::MONTHLYDOW => (t(&MONTHLYDOW_TRIGGER.0), iid::MONTHLYDOW_TRIGGER),
        trigger::BOOT => (t(&BOOT_TRIGGER.0), iid::BOOT_TRIGGER),
        _ => (t(&REGISTRATION_TRIGGER.0), iid::REGISTRATION_TRIGGER),
    }
}
fn action_table(kind: u32) -> (*const (), Guid) {
    let t = |table: *const [*const ()]| table as *const ();
    match kind {
        action::EXEC => (t(&EXEC.0), iid::EXEC),
        action::COM_HANDLER => (t(&COM_HANDLER.0), iid::COM_HANDLER),
        action::SEND_EMAIL => (t(&EMAIL.0), iid::EMAIL),
        _ => (t(&SHOW_MESSAGE.0), iid::SHOW_MESSAGE),
    }
}
static TRIGGER_IIDS: [[Guid; 2]; 12] = {
    let mut a = [[iid::TRIGGER; 2]; 12];
    a[0][1] = iid::EVENT_TRIGGER;
    a[1][1] = iid::TIME_TRIGGER;
    a[2][1] = iid::DAILY_TRIGGER;
    a[3][1] = iid::WEEKLY_TRIGGER;
    a[4][1] = iid::MONTHLY_TRIGGER;
    a[5][1] = iid::MONTHLYDOW_TRIGGER;
    a[6][1] = iid::IDLE_TRIGGER;
    a[7][1] = iid::REGISTRATION_TRIGGER;
    a[8][1] = iid::BOOT_TRIGGER;
    a[9][1] = iid::LOGON_TRIGGER;
    a[10][1] = iid::SESSION_TRIGGER;
    a[11][1] = iid::SESSION_TRIGGER;
    a
};
static EXEC_IIDS: [Guid; 3] = [iid::ACTION, iid::EXEC, iid::EXEC2];
static COM_IIDS: [Guid; 2] = [iid::ACTION, iid::COM_HANDLER];
static EMAIL_IIDS: [Guid; 2] = [iid::ACTION, iid::EMAIL];
static MESSAGE_IIDS: [Guid; 2] = [iid::ACTION, iid::SHOW_MESSAGE];
fn raw_obj(vtbl: *const (), iid: Guid, primary: &'static [Guid], kind: Kind) -> u64 {
    Box::into_raw(Box::new(Obj {
        vtbl,
        second: core::ptr::null(),
        refs: AtomicU32::new(1),
        iid,
        primary,
        secondary: &[],
        second_iid: [0; 16],
        kind,
    })) as u64
}
/// The item's COM object (made when it has none): a new reference.
fn trigger_object(item: &Arc<Item<Trigger>>) -> u64 {
    item.with(|(t, object)| {
        if *object != 0 {
            add_ref(*object);
            return *object;
        }
        let (table, iid) = trigger_table(t.kind);
        let index = if t.kind == trigger::SESSION_STATE_CHANGE {
            10
        } else {
            (t.kind as usize).min(11)
        };
        let p = raw_obj(
            table,
            iid,
            &TRIGGER_IIDS[index],
            Kind::Def(Part::Trigger(item.clone())),
        );
        *object = p;
        p
    })
}
fn action_object(item: &Arc<Item<Action>>) -> u64 {
    item.with(|(a, object)| {
        if *object != 0 {
            add_ref(*object);
            return *object;
        }
        let (table, iid) = action_table(a.kind);
        let primary: &'static [Guid] = match a.kind {
            action::EXEC => &EXEC_IIDS,
            action::COM_HANDLER => &COM_IIDS,
            action::SEND_EMAIL => &EMAIL_IIDS,
            _ => &MESSAGE_IIDS,
        };
        let p = raw_obj(table, iid, primary, Kind::Def(Part::Action(item.clone())));
        *object = p;
        p
    })
}
/// Release of a trigger or action object: under its item's lock, so that a
/// collection never hands out an object being freed.
pub(crate) fn release_item(this: u64) -> Option<u32> {
    let o = obj(this);
    let drop_now = |left: u32| {
        if left == 0 {
            // SAFETY: the last reference; the object came from Box::into_raw.
            drop(unsafe { Box::from_raw(this as *mut Obj) });
        }
        left
    };
    match &o.kind {
        Kind::Def(Part::Trigger(item)) => {
            let left = item.with(|(_, object)| {
                let left = o.refs.fetch_sub(1, Ordering::AcqRel) - 1;
                if left == 0 && *object == this {
                    *object = 0;
                }
                left
            });
            Some(drop_now(left))
        }
        Kind::Def(Part::Action(item)) => {
            let left = item.with(|(_, object)| {
                let left = o.refs.fetch_sub(1, Ordering::AcqRel) - 1;
                if left == 0 && *object == this {
                    *object = 0;
                }
                left
            });
            Some(drop_now(left))
        }
        _ => None,
    }
}

// ---- making part objects ---------------------------------------------------------------
fn make(p: Part) -> u64 {
    let (table, iid, primary): (*const (), Guid, &'static [Guid]) = match &p {
        Part::Definition(_) => (
            DEFINITION.0.as_ptr() as *const (),
            iid::DEFINITION,
            &[iid::DEFINITION],
        ),
        Part::Info(_) => (
            INFO.0.as_ptr() as *const (),
            iid::REGISTRATION_INFO,
            &[iid::REGISTRATION_INFO],
        ),
        Part::Triggers(_) => (
            TRIGGERS.0.as_ptr() as *const (),
            iid::TRIGGERS,
            &[iid::TRIGGERS],
        ),
        Part::Repetition(_) => (
            REPETITION.0.as_ptr() as *const (),
            iid::REPETITION,
            &[iid::REPETITION],
        ),
        Part::Actions(_) => (
            ACTIONS.0.as_ptr() as *const (),
            iid::ACTIONS,
            &[iid::ACTIONS],
        ),
        Part::Settings(_) => (
            SETTINGS.0.as_ptr() as *const (),
            iid::SETTINGS3,
            &[iid::SETTINGS, iid::SETTINGS3],
        ),
        Part::Idle(_) => (IDLE.0.as_ptr() as *const (), iid::IDLE, &[iid::IDLE]),
        Part::Network(_) => (
            NETWORK.0.as_ptr() as *const (),
            iid::NETWORK,
            &[iid::NETWORK],
        ),
        Part::Maintenance(_) => (
            MAINTENANCE.0.as_ptr() as *const (),
            iid::MAINTENANCE,
            &[iid::MAINTENANCE],
        ),
        Part::Principal(_) => (
            PRINCIPAL.0.as_ptr() as *const (),
            iid::PRINCIPAL,
            &[iid::PRINCIPAL],
        ),
        Part::TriggerPairs(_) | Part::ActionPairs(_) => {
            (PAIRS.0.as_ptr() as *const (), iid::PAIRS, &[iid::PAIRS])
        }
        Part::Pair(_) => (PAIR.0.as_ptr() as *const (), iid::PAIR, &[iid::PAIR]),
        Part::Trigger(item) => return trigger_object(item),
        Part::Action(item) => return action_object(item),
    };
    let second = match &p {
        Part::Settings(_) => Some((SETTINGS2.0.as_ptr() as *const (), iid::SETTINGS2)),
        Part::Principal(_) => Some((PRINCIPAL2.0.as_ptr() as *const (), iid::PRINCIPAL2)),
        _ => None,
    };
    let o = raw_obj(table, iid, primary, Kind::Def(p));
    if let Some((table, iid)) = second {
        // SAFETY: just made, not shared yet.
        let x = unsafe { &mut *(o as *mut Obj) };
        x.second = table;
        x.second_iid = iid;
        x.secondary = if iid == iid::SETTINGS2 {
            &[iid::SETTINGS2]
        } else {
            &[iid::PRINCIPAL2]
        };
    }
    o
}
/// A part object out-parameter.
fn out_part(out: u64, p: Part) -> u32 {
    if out == 0 {
        return hr::E_POINTER;
    }
    result(write_pointer(out, make(p)))
}
pub(crate) fn is_our_table(t: u64) -> bool {
    let tables: [u64; 26] = [
        DEFINITION.0.as_ptr() as u64,
        INFO.0.as_ptr() as u64,
        TRIGGERS.0.as_ptr() as u64,
        REPETITION.0.as_ptr() as u64,
        ACTIONS.0.as_ptr() as u64,
        SETTINGS.0.as_ptr() as u64,
        SETTINGS2.0.as_ptr() as u64,
        IDLE.0.as_ptr() as u64,
        NETWORK.0.as_ptr() as u64,
        MAINTENANCE.0.as_ptr() as u64,
        PRINCIPAL.0.as_ptr() as u64,
        PRINCIPAL2.0.as_ptr() as u64,
        PAIRS.0.as_ptr() as u64,
        PAIR.0.as_ptr() as u64,
        IDLE_TRIGGER.0.as_ptr() as u64,
        LOGON_TRIGGER.0.as_ptr() as u64,
        SESSION_TRIGGER.0.as_ptr() as u64,
        EVENT_TRIGGER.0.as_ptr() as u64,
        TIME_TRIGGER.0.as_ptr() as u64,
        DAILY_TRIGGER.0.as_ptr() as u64,
        WEEKLY_TRIGGER.0.as_ptr() as u64,
        MONTHLY_TRIGGER.0.as_ptr() as u64,
        MONTHLYDOW_TRIGGER.0.as_ptr() as u64,
        BOOT_TRIGGER.0.as_ptr() as u64,
        REGISTRATION_TRIGGER.0.as_ptr() as u64,
        EXEC.0.as_ptr() as u64,
    ];
    tables.contains(&t)
        || [
            COM_HANDLER.0.as_ptr() as u64,
            EMAIL.0.as_ptr() as u64,
            SHOW_MESSAGE.0.as_ptr() as u64,
        ]
        .contains(&t)
}

// ---- fields ------------------------------------------------------------------------------
pub(crate) mod f {
    pub const DESCRIPTION: u32 = 0;
    pub const AUTHOR: u32 = 1;
    pub const VERSION: u32 = 2;
    pub const DATE: u32 = 3;
    pub const DOCUMENTATION: u32 = 4;
    pub const URI: u32 = 5;
    pub const SOURCE: u32 = 6;
    pub const SECURITY: u32 = 7;
    pub const T_ID: u32 = 10;
    pub const T_LIMIT: u32 = 11;
    pub const T_START: u32 = 12;
    pub const T_END: u32 = 13;
    pub const T_RANDOM: u32 = 14;
    pub const T_DELAY: u32 = 15;
    pub const T_USER: u32 = 16;
    pub const T_SUBSCRIPTION: u32 = 17;
    pub const R_INTERVAL: u32 = 20;
    pub const R_DURATION: u32 = 21;
    pub const A_ID: u32 = 30;
    pub const A_PATH: u32 = 31;
    pub const A_ARGUMENTS: u32 = 32;
    pub const A_DIRECTORY: u32 = 33;
    pub const A_CLASS: u32 = 34;
    pub const A_DATA: u32 = 35;
    pub const A_SERVER: u32 = 36;
    pub const A_SUBJECT: u32 = 37;
    pub const A_TO: u32 = 38;
    pub const A_CC: u32 = 39;
    pub const A_BCC: u32 = 40;
    pub const A_REPLY_TO: u32 = 41;
    pub const A_FROM: u32 = 42;
    pub const A_BODY: u32 = 43;
    pub const A_TITLE: u32 = 44;
    pub const S_RESTART: u32 = 50;
    pub const S_LIMIT: u32 = 51;
    pub const S_DELETE: u32 = 52;
    pub const I_DURATION: u32 = 53;
    pub const I_WAIT: u32 = 54;
    pub const N_NAME: u32 = 55;
    pub const N_ID: u32 = 56;
    pub const M_PERIOD: u32 = 57;
    pub const M_DEADLINE: u32 = 58;
    pub const P_ID: u32 = 60;
    pub const P_DISPLAY: u32 = 61;
    pub const P_USER: u32 = 62;
    pub const P_GROUP: u32 = 63;
    pub const PAIR_NAME: u32 = 70;
    pub const PAIR_VALUE: u32 = 71;
    pub const DATA: u32 = 80;
    pub const CONTEXT: u32 = 81;
    pub const S_DEMAND: u32 = 100;
    pub const S_STOP_BATTERIES: u32 = 101;
    pub const S_DISALLOW_BATTERIES: u32 = 102;
    pub const S_HARD: u32 = 103;
    pub const S_WHEN_AVAILABLE: u32 = 104;
    pub const S_NETWORK: u32 = 105;
    pub const S_ENABLED: u32 = 106;
    pub const S_HIDDEN: u32 = 107;
    pub const S_IDLE_ONLY: u32 = 108;
    pub const S_WAKE: u32 = 109;
    pub const S_REMOTE_APP: u32 = 110;
    pub const S_UNIFIED: u32 = 111;
    pub const S_VOLATILE: u32 = 112;
    pub const I_STOP: u32 = 113;
    pub const I_RESTART: u32 = 114;
    pub const M_EXCLUSIVE: u32 = 115;
    pub const T_ENABLED: u32 = 120;
    pub const R_STOP: u32 = 121;
    pub const T_LAST_DAY: u32 = 122;
    pub const T_LAST_WEEK: u32 = 123;
    pub const A_HIDE: u32 = 125;
    pub const S_RESTART_COUNT: u32 = 130;
    pub const S_INSTANCES: u32 = 131;
    pub const S_PRIORITY: u32 = 132;
    pub const S_COMPATIBILITY: u32 = 133;
    pub const P_LOGON: u32 = 134;
    pub const P_RUN_LEVEL: u32 = 135;
    pub const P_SID_TYPE: u32 = 136;
    pub const T_DAYS_INTERVAL: u32 = 140;
    pub const T_DAYS_OF_WEEK: u32 = 141;
    pub const T_WEEKS_INTERVAL: u32 = 142;
    pub const T_MONTHS: u32 = 143;
    pub const T_WEEKS_OF_MONTH: u32 = 144;
    pub const T_DAYS_OF_MONTH: u32 = 145;
    pub const T_STATE_CHANGE: u32 = 146;
}
fn info_slot(i: &mut Registration, field: u32) -> Option<&mut Option<String>> {
    Some(match field {
        f::DESCRIPTION => &mut i.description,
        f::AUTHOR => &mut i.author,
        f::VERSION => &mut i.version,
        f::DATE => &mut i.date,
        f::DOCUMENTATION => &mut i.documentation,
        f::URI => &mut i.uri,
        f::SOURCE => &mut i.source,
        f::SECURITY => &mut i.security_descriptor,
        _ => return None,
    })
}
fn trigger_slot(t: &mut Trigger, field: u32) -> Option<&mut Option<String>> {
    Some(match field {
        f::T_ID => &mut t.id,
        f::T_LIMIT => &mut t.limit,
        f::T_START => &mut t.start,
        f::T_END => &mut t.end,
        f::T_RANDOM => &mut t.random_delay,
        f::T_DELAY => &mut t.delay,
        f::T_USER => &mut t.user_id,
        f::T_SUBSCRIPTION => &mut t.subscription,
        f::R_INTERVAL => &mut t.repetition.interval,
        f::R_DURATION => &mut t.repetition.duration,
        _ => return None,
    })
}
fn action_slot(a: &mut Action, field: u32) -> Option<&mut Option<String>> {
    Some(match field {
        f::A_ID => &mut a.id,
        f::A_PATH => &mut a.path,
        f::A_ARGUMENTS => &mut a.arguments,
        f::A_DIRECTORY => &mut a.directory,
        f::A_CLASS => &mut a.class_id,
        f::A_DATA => &mut a.data,
        f::A_SERVER => &mut a.server,
        f::A_SUBJECT => &mut a.subject,
        f::A_TO => &mut a.to,
        f::A_CC => &mut a.cc,
        f::A_BCC => &mut a.bcc,
        f::A_REPLY_TO => &mut a.reply_to,
        f::A_FROM => &mut a.from,
        f::A_BODY => &mut a.body,
        f::A_TITLE => &mut a.title,
        _ => return None,
    })
}
fn parts_slot(p: &mut Parts, field: u32) -> Option<&mut Option<String>> {
    Some(match field {
        f::S_RESTART => &mut p.settings.restart_interval,
        f::S_LIMIT => &mut p.settings.limit,
        f::S_DELETE => &mut p.settings.delete_after,
        f::I_DURATION => &mut p.settings.idle.duration,
        f::I_WAIT => &mut p.settings.idle.wait,
        f::N_NAME => &mut p.settings.network_name,
        f::N_ID => &mut p.settings.network_id,
        f::M_PERIOD => &mut p.maintenance.get_or_insert((None, None, false)).0,
        f::M_DEADLINE => &mut p.maintenance.get_or_insert((None, None, false)).1,
        f::P_ID => &mut p.principal.id,
        f::P_DISPLAY => &mut p.principal.display_name,
        f::P_USER => &mut p.principal.user_id,
        f::P_GROUP => &mut p.principal.group_id,
        f::DATA => &mut p.data,
        f::CONTEXT => &mut p.context,
        f => return info_slot(&mut p.info, f),
    })
}
/// Runs `f` on string field `field` of the object.
fn with_str<R>(this: u64, field: u32, f: impl FnOnce(&mut Option<String>) -> R) -> Result<R, u32> {
    match part(this).ok_or(hr::E_FAIL)? {
        Part::Trigger(item) | Part::Repetition(item) => item
            .with(|(t, _)| trigger_slot(t, field).map(f))
            .ok_or(hr::E_FAIL),
        Part::Action(item) => item
            .with(|(a, _)| action_slot(a, field).map(f))
            .ok_or(hr::E_FAIL),
        Part::Pair(pair) => pair.with(|(name, value)| {
            let mut v = Some(if field == f::PAIR_NAME {
                name.clone()
            } else {
                value.clone()
            });
            let r = f(&mut v);
            let text = v.unwrap_or_default();
            if field == f::PAIR_NAME {
                *name = text;
            } else {
                *value = text;
            }
            Ok(r)
        }),
        _ => {
            let def = def_of(this).ok_or(hr::E_FAIL)?;
            def.parts
                .with(|p| parts_slot(p, field).map(f))
                .ok_or(hr::E_FAIL)
        }
    }
}
/// Boolean field `field`: its value, and (Some(v)) the new one.
fn with_bool(this: u64, field: u32, set: Option<bool>) -> Result<bool, u32> {
    let apply = |slot: &mut bool| {
        if let Some(v) = set {
            *slot = v;
        }
        *slot
    };
    match part(this).ok_or(hr::E_FAIL)? {
        Part::Trigger(item) | Part::Repetition(item) => item.with(|(t, _)| {
            Ok(apply(match field {
                f::T_ENABLED => &mut t.enabled,
                f::R_STOP => &mut t.repetition.stop_at_end,
                f::T_LAST_DAY => &mut t.last_day,
                f::T_LAST_WEEK => &mut t.last_week,
                _ => return Err(hr::E_FAIL),
            }))
        }),
        Part::Action(item) => item.with(|(a, _)| match field {
            f::A_HIDE => Ok(apply(&mut a.hide_window)),
            _ => Err(hr::E_FAIL),
        }),
        _ => {
            let def = def_of(this).ok_or(hr::E_FAIL)?;
            def.parts.with(|p| {
                let s = &mut p.settings;
                Ok(apply(match field {
                    f::S_DEMAND => &mut s.demand_start,
                    f::S_STOP_BATTERIES => &mut s.stop_on_batteries,
                    f::S_DISALLOW_BATTERIES => &mut s.disallow_on_batteries,
                    f::S_HARD => &mut s.hard_terminate,
                    f::S_WHEN_AVAILABLE => &mut s.when_available,
                    f::S_NETWORK => &mut s.network_only,
                    f::S_ENABLED => &mut s.enabled,
                    f::S_HIDDEN => &mut s.hidden,
                    f::S_IDLE_ONLY => &mut s.idle_only,
                    f::S_WAKE => &mut s.wake,
                    f::S_REMOTE_APP => &mut s.remote_app,
                    f::S_UNIFIED => &mut s.unified,
                    f::S_VOLATILE => &mut s.volatile,
                    f::I_STOP => &mut s.idle.stop_on_end,
                    f::I_RESTART => &mut s.idle.restart,
                    f::M_EXCLUSIVE => &mut p.maintenance.get_or_insert((None, None, false)).2,
                    _ => return Err(hr::E_FAIL),
                }))
            })
        }
    }
}
/// Integer field `field`: its value, and (Some(v)) the new one, checked
/// as Windows checks it.
fn with_int(this: u64, field: u32, set: Option<i32>) -> Result<i32, u32> {
    let check = |v: i32, ok: bool| if ok { Ok(v) } else { Err(hr::E_INVALIDARG) };
    match part(this).ok_or(hr::E_FAIL)? {
        Part::Trigger(item) => item.with(|(t, _)| {
            macro_rules! field {
                ($slot:expr, $ty:ty, $ok:expr) => {{
                    if let Some(v) = set {
                        $slot = check(v, $ok(v))? as $ty;
                    }
                    Ok(i32::from($slot))
                }};
            }
            match field {
                f::T_DAYS_INTERVAL => field!(t.days_interval, i16, |v: i32| v > 0 && v <= 365),
                f::T_DAYS_OF_WEEK => field!(t.days_of_week, i16, |v: i32| (0..=0x7F).contains(&v)),
                f::T_WEEKS_INTERVAL => field!(t.weeks_interval, i16, |v: i32| v > 0 && v <= 52),
                f::T_MONTHS => field!(t.months, i16, |v: i32| (0..=0xFFF).contains(&v)),
                f::T_WEEKS_OF_MONTH => {
                    field!(t.weeks_of_month, i16, |v: i32| (0..=0xF).contains(&v))
                }
                f::T_DAYS_OF_MONTH => {
                    if let Some(v) = set {
                        t.days_of_month = check(v, v as u32 & 0x8000_0000 == 0)?;
                    }
                    Ok(t.days_of_month)
                }
                f::T_STATE_CHANGE => {
                    if let Some(v) = set {
                        t.state_change = check(v, matches!(v, 0..=4 | 7 | 8))? as u32;
                    }
                    Ok(t.state_change as i32)
                }
                _ => Err(hr::E_FAIL),
            }
        }),
        _ => {
            let def = def_of(this).ok_or(hr::E_FAIL)?;
            def.parts.with(|p| {
                let (slot, ok): (&mut i32, fn(i32) -> bool) = match field {
                    f::S_RESTART_COUNT => (&mut p.settings.restart_count, |v| v >= 0),
                    f::S_PRIORITY => (&mut p.settings.priority, |v| (0..=10).contains(&v)),
                    f::P_SID_TYPE => (&mut p.sid_type, |v| (0..=2).contains(&v)),
                    f::S_INSTANCES | f::S_COMPATIBILITY | f::P_LOGON | f::P_RUN_LEVEL => {
                        let (slot, ok): (&mut u32, fn(i32) -> bool) = match field {
                            f::S_INSTANCES => (&mut p.settings.instances, |_| true),
                            f::S_COMPATIBILITY => {
                                (&mut p.settings.compatibility, |v| (0..=6).contains(&v))
                            }
                            f::P_LOGON => (&mut p.principal.logon, |v| (0..=6).contains(&v)),
                            _ => (&mut p.principal.run_level, |v| (0..=1).contains(&v)),
                        };
                        if let Some(v) = set {
                            *slot = check(v, ok(v))? as u32;
                            if field == f::P_LOGON {
                                p.principal.logon_set = true;
                            }
                        }
                        return Ok(*slot as i32);
                    }
                    _ => return Err(hr::E_FAIL),
                };
                if let Some(v) = set {
                    *slot = check(v, ok(v))?;
                }
                Ok(*slot)
            })
        }
    }
}
// Generic accessors: `S` strings, `B` VARIANT_BOOLs, `L` longs, `H`
// shorts; OFF is 8 for the secondary interface.
extern "system" fn gs<const F: u32>(this: usize, out: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let out = door_windows_abi::widen(out);
    {
        if out == 0 {
            return hr::E_POINTER;
        }
        match with_str(this, F, |s| s.clone()) {
            Ok(v) => bstr_out(out, v.as_deref()),
            Err(e) => e,
        }
    }
}
extern "system" fn ps<const F: u32>(this: usize, value: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let value = door_windows_abi::widen(value);
    {
        let v = bstr_in(value);
        result(with_str(this, F, |s| *s = v))
    }
}
extern "system" fn gb<const F: u32, const OFF: u64>(this: usize, out: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let out = door_windows_abi::widen(out);
    {
        match with_bool(this - OFF, F, None) {
            Ok(v) => result(write_u16(out, if v { 0xFFFF } else { 0 })),
            Err(e) => e,
        }
    }
}
extern "system" fn pb<const F: u32, const OFF: u64>(this: usize, value: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let value = door_windows_abi::widen(value);
    result(with_bool(this - OFF, F, Some(value as u16 != 0)).map(|_| ()))
}
extern "system" fn gl<const F: u32, const OFF: u64>(this: usize, out: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let out = door_windows_abi::widen(out);
    {
        match with_int(this - OFF, F, None) {
            Ok(v) => result(write_u32(out, v as u32)),
            Err(e) => e,
        }
    }
}
extern "system" fn pl<const F: u32, const OFF: u64>(this: usize, value: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let value = door_windows_abi::widen(value);
    result(with_int(this - OFF, F, Some(value as u32 as i32)).map(|_| ()))
}
extern "system" fn gh<const F: u32>(this: usize, out: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let out = door_windows_abi::widen(out);
    {
        match with_int(this, F, None) {
            Ok(v) => result(write_u16(out, v as u16)),
            Err(e) => e,
        }
    }
}
extern "system" fn ph<const F: u32>(this: usize, value: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let value = door_windows_abi::widen(value);
    result(with_int(this, F, Some(i32::from(value as u16 as i16))).map(|_| ()))
}
/// Every part's XmlText: the whole definition's XML.
extern "system" fn get_xml(this: usize, out: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let out = door_windows_abi::widen(out);
    {
        match def_of(this) {
            Some(d) => bstr_out(out, Some(&xml::write(&snapshot(d)))),
            None => hr::E_FAIL,
        }
    }
}
/// put_XmlText of the definition replaces all of it; of a part, that part.
extern "system" fn put_xml(this: usize, text: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let text = door_windows_abi::widen(text);
    {
        let Some(d) = def_of(this) else {
            return hr::E_FAIL;
        };
        let Some(text) = bstr_in(text) else {
            return hr::E_INVALIDARG;
        };
        let new = match xml::read(&text) {
            Ok(n) => n,
            Err(e) => return e,
        };
        match part(this) {
            Some(Part::Definition(_)) => load(d, new),
            Some(Part::Info(_)) => d.parts.with(|p| p.info = new.info),
            Some(Part::Settings(_)) => d.parts.with(|p| p.settings = new.settings),
            Some(Part::Principal(_)) => d.parts.with(|p| p.principal = new.principal),
            Some(Part::Actions(_)) => d.parts.with(|p| {
                p.actions = new
                    .actions
                    .into_iter()
                    .map(|a| Arc::new(Lock::new((a, 0))))
                    .collect();
                p.context = new.context;
            }),
            _ => return hr::E_NOTIMPL,
        }
        hr::S_OK
    }
}

// ---- ITaskDefinition ---------------------------------------------------------------------
static DEFINITION: Table<21> = dispatch1!(
    def_info,
    def_put_info,
    def_triggers,
    def_put_triggers,
    def_settings,
    def_put_settings,
    gs::<{ f::DATA }>,
    ps::<{ f::DATA }>,
    def_principal,
    def_put_principal,
    def_actions,
    def_put_actions,
    get_xml,
    put_xml,
);
fn the_def(this: u64) -> Result<&'static Arc<Def>, u32> {
    def_of(this).ok_or(hr::E_FAIL)
}
extern "system" fn def_info(this: usize, out: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let out = door_windows_abi::widen(out);
    {
        match the_def(this) {
            Ok(d) => out_part(out, Part::Info(d.clone())),
            Err(e) => e,
        }
    }
}
/// The other definition behind one of our part pointers.
fn other(p: u64) -> Result<&'static Arc<Def>, u32> {
    if !ours(p) {
        return Err(hr::E_INVALIDARG);
    }
    def_of(p).ok_or(hr::E_INVALIDARG)
}
extern "system" fn def_put_info(this: usize, p: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let p = door_windows_abi::widen(p);
    {
        let (d, o) = match (the_def(this), other(p)) {
            (Ok(d), Ok(o)) => (d, o),
            (Err(e), _) | (_, Err(e)) => return e,
        };
        let info = o.parts.with(|x| x.info.clone());
        d.parts.with(|x| x.info = info);
        hr::S_OK
    }
}
extern "system" fn def_triggers(this: usize, out: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let out = door_windows_abi::widen(out);
    {
        match the_def(this) {
            Ok(d) => out_part(out, Part::Triggers(d.clone())),
            Err(e) => e,
        }
    }
}
extern "system" fn def_put_triggers(this: usize, p: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let p = door_windows_abi::widen(p);
    {
        let (d, o) = match (the_def(this), other(p)) {
            (Ok(d), Ok(o)) => (d, o),
            (Err(e), _) | (_, Err(e)) => return e,
        };
        let items = o.parts.with(|x| x.triggers.clone());
        d.parts.with(|x| x.triggers = items);
        hr::S_OK
    }
}
extern "system" fn def_settings(this: usize, out: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let out = door_windows_abi::widen(out);
    {
        match the_def(this) {
            Ok(d) => out_part(out, Part::Settings(d.clone())),
            Err(e) => e,
        }
    }
}
extern "system" fn def_put_settings(this: usize, p: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let p = door_windows_abi::widen(p);
    {
        let (d, o) = match (the_def(this), other(p)) {
            (Ok(d), Ok(o)) => (d, o),
            (Err(e), _) | (_, Err(e)) => return e,
        };
        let s = o.parts.with(|x| x.settings.clone());
        d.parts.with(|x| x.settings = s);
        hr::S_OK
    }
}
extern "system" fn def_principal(this: usize, out: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let out = door_windows_abi::widen(out);
    {
        match the_def(this) {
            Ok(d) => out_part(out, Part::Principal(d.clone())),
            Err(e) => e,
        }
    }
}
extern "system" fn def_put_principal(this: usize, p: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let p = door_windows_abi::widen(p);
    {
        let (d, o) = match (the_def(this), other(p)) {
            (Ok(d), Ok(o)) => (d, o),
            (Err(e), _) | (_, Err(e)) => return e,
        };
        let s = o.parts.with(|x| x.principal.clone());
        d.parts.with(|x| x.principal = s);
        hr::S_OK
    }
}
extern "system" fn def_actions(this: usize, out: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let out = door_windows_abi::widen(out);
    {
        match the_def(this) {
            Ok(d) => out_part(out, Part::Actions(d.clone())),
            Err(e) => e,
        }
    }
}
extern "system" fn def_put_actions(this: usize, p: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let p = door_windows_abi::widen(p);
    {
        let (d, o) = match (the_def(this), other(p)) {
            (Ok(d), Ok(o)) => (d, o),
            (Err(e), _) | (_, Err(e)) => return e,
        };
        let (items, context) = o.parts.with(|x| (x.actions.clone(), x.context.clone()));
        d.parts.with(|x| {
            x.actions = items;
            x.context = context;
        });
        hr::S_OK
    }
}

// ---- IRegistrationInfo -------------------------------------------------------------------
static INFO: Table<25> = dispatch1!(
    gs::<{ f::DESCRIPTION }>,
    ps::<{ f::DESCRIPTION }>,
    gs::<{ f::AUTHOR }>,
    ps::<{ f::AUTHOR }>,
    gs::<{ f::VERSION }>,
    ps::<{ f::VERSION }>,
    gs::<{ f::DATE }>,
    ps::<{ f::DATE }>,
    gs::<{ f::DOCUMENTATION }>,
    ps::<{ f::DOCUMENTATION }>,
    get_xml,
    put_xml,
    gs::<{ f::URI }>,
    ps::<{ f::URI }>,
    info_get_sd,
    info_put_sd,
    gs::<{ f::SOURCE }>,
    ps::<{ f::SOURCE }>,
);
extern "system" fn info_get_sd(this: usize, out: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let out = door_windows_abi::widen(out);
    {
        match with_str(this, f::SECURITY, |s| s.clone()) {
            Ok(v) => variant_out(out, v.as_deref()),
            Err(e) => e,
        }
    }
}
extern "system" fn info_put_sd(this: usize, value: VariantArg) -> u32 {
    let this = door_windows_abi::widen(this);
    let value = value.at();
    {
        let v = match variant(value) {
            Ok(Arg::Text(t)) => Some(t),
            Ok(Arg::Empty) => None,
            Ok(Arg::Number(_)) => return hr::E_INVALIDARG,
            Err(e) => return e,
        };
        result(with_str(this, f::SECURITY, |s| *s = v))
    }
}

// ---- triggers -----------------------------------------------------------------------------
static TRIGGERS: Table<13> = dispatch1!(
    triggers_count,
    triggers_item,
    triggers_enum,
    triggers_create,
    triggers_remove,
    triggers_clear,
);
extern "system" fn triggers_count(this: usize, out: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let out = door_windows_abi::widen(out);
    {
        match the_def(this) {
            Ok(d) => result(write_u32(out, d.parts.with(|p| p.triggers.len()) as u32)),
            Err(e) => e,
        }
    }
}
extern "system" fn triggers_item(this: usize, index: i32, out: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let out = door_windows_abi::widen(out);
    {
        if out == 0 {
            return hr::E_POINTER;
        }
        let _ = write_pointer(out, 0);
        let Ok(d) = the_def(this) else {
            return hr::E_FAIL;
        };
        if index < 1 {
            return hr::E_INVALIDARG;
        }
        // Past the last: E_FAIL (measured).
        match d
            .parts
            .with(|p| p.triggers.get(index as usize - 1).cloned())
        {
            Some(item) => result(write_pointer(out, trigger_object(&item))),
            None => hr::E_FAIL,
        }
    }
}
extern "system" fn triggers_enum(this: usize, out: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let out = door_windows_abi::widen(out);
    {
        let Ok(d) = the_def(this) else {
            return hr::E_FAIL;
        };
        let items: Vec<u64> = d
            .parts
            .with(|p| p.triggers.clone())
            .iter()
            .map(trigger_object)
            .collect();
        let e = new_enum(items.clone());
        for i in items {
            release(i);
        }
        result(write_pointer(out, e))
    }
}
extern "system" fn triggers_create(this: usize, kind: u32, out: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let out = door_windows_abi::widen(out);
    {
        if out == 0 {
            return hr::E_POINTER;
        }
        let _ = write_pointer(out, 0);
        if !trigger::creatable(kind) {
            return hr::E_INVALIDARG;
        }
        let Ok(d) = the_def(this) else {
            return hr::E_FAIL;
        };
        let item = Arc::new(Lock::new((Trigger::new(kind), 0)));
        d.parts.with(|p| p.triggers.push(item.clone()));
        result(write_pointer(out, trigger_object(&item)))
    }
}
/// Remove by 1-based index, or by id.
fn remove_at<T>(
    list: &mut Vec<Arc<Item<T>>>,
    which: Arg,
    id: impl Fn(&T) -> Option<&String>,
) -> u32 {
    let at = match which {
        Arg::Number(n) if n >= 1 && (n as usize) <= list.len() => n as usize - 1,
        Arg::Text(name) => match list
            .iter()
            .position(|i| i.with(|(v, _)| id(v).is_some_and(|x| *x == name)))
        {
            Some(i) => i,
            None => return hr::E_INVALIDARG,
        },
        _ => return hr::E_INVALIDARG,
    };
    list.remove(at);
    hr::S_OK
}
extern "system" fn triggers_remove(this: usize, which: VariantArg) -> u32 {
    let this = door_windows_abi::widen(this);
    let which = which.at();
    {
        let (Ok(d), Ok(which)) = (the_def(this), variant(which)) else {
            return hr::E_INVALIDARG;
        };
        d.parts
            .with(|p| remove_at(&mut p.triggers, which, |t| t.id.as_ref()))
    }
}
extern "system" fn triggers_clear(this: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    {
        match the_def(this) {
            Ok(d) => {
                d.parts.with(|p| p.triggers.clear());
                hr::S_OK
            }
            Err(e) => e,
        }
    }
}
fn item_of(this: u64) -> Option<&'static Arc<Item<Trigger>>> {
    match part(this)? {
        Part::Trigger(i) | Part::Repetition(i) => Some(i),
        _ => None,
    }
}
extern "system" fn trigger_type(this: usize, out: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let out = door_windows_abi::widen(out);
    {
        match item_of(this) {
            Some(i) => result(write_u32(out, i.with(|(t, _)| t.kind))),
            None => hr::E_FAIL,
        }
    }
}
extern "system" fn trigger_repetition(this: usize, out: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let out = door_windows_abi::widen(out);
    {
        match item_of(this) {
            Some(i) => out_part(out, Part::Repetition(i.clone())),
            None => hr::E_FAIL,
        }
    }
}
extern "system" fn trigger_put_repetition(this: usize, p: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let p = door_windows_abi::widen(p);
    {
        let (Some(i), true) = (item_of(this), ours(p)) else {
            return hr::E_INVALIDARG;
        };
        let Some(from) = item_of(p) else {
            return hr::E_INVALIDARG;
        };
        let r = from.with(|(t, _)| t.repetition.clone());
        i.with(|(t, _)| t.repetition = r);
        hr::S_OK
    }
}
extern "system" fn trigger_pairs(this: usize, out: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let out = door_windows_abi::widen(out);
    {
        match item_of(this) {
            Some(i) => out_part(out, Part::TriggerPairs(i.clone())),
            None => hr::E_FAIL,
        }
    }
}
extern "system" fn trigger_put_pairs(this: usize, p: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let p = door_windows_abi::widen(p);
    {
        let Some(i) = item_of(this) else {
            return hr::E_FAIL;
        };
        let pairs = match pairs_of(p) {
            Ok(v) => v,
            Err(e) => return e,
        };
        i.with(|(t, _)| t.value_queries = pairs);
        hr::S_OK
    }
}
/// The 13 entries of ITrigger after IDispatch.
macro_rules! trigger_table {
    ($($rest:expr),* $(,)?) => {
        dispatch1!(
            trigger_type,
            gs::<{ f::T_ID }>,
            ps::<{ f::T_ID }>,
            trigger_repetition,
            trigger_put_repetition,
            gs::<{ f::T_LIMIT }>,
            ps::<{ f::T_LIMIT }>,
            gs::<{ f::T_START }>,
            ps::<{ f::T_START }>,
            gs::<{ f::T_END }>,
            ps::<{ f::T_END }>,
            gb::<{ f::T_ENABLED }, 0>,
            pb::<{ f::T_ENABLED }, 0>,
            $($rest),*
        )
    };
}
static IDLE_TRIGGER: Table<20> = trigger_table!();
static LOGON_TRIGGER: Table<24> = trigger_table!(
    gs::<{ f::T_DELAY }>,
    ps::<{ f::T_DELAY }>,
    gs::<{ f::T_USER }>,
    ps::<{ f::T_USER }>,
);
static SESSION_TRIGGER: Table<26> = trigger_table!(
    gs::<{ f::T_DELAY }>,
    ps::<{ f::T_DELAY }>,
    gs::<{ f::T_USER }>,
    ps::<{ f::T_USER }>,
    gl::<{ f::T_STATE_CHANGE }, 0>,
    pl::<{ f::T_STATE_CHANGE }, 0>,
);
static EVENT_TRIGGER: Table<26> = trigger_table!(
    gs::<{ f::T_SUBSCRIPTION }>,
    ps::<{ f::T_SUBSCRIPTION }>,
    gs::<{ f::T_DELAY }>,
    ps::<{ f::T_DELAY }>,
    trigger_pairs,
    trigger_put_pairs,
);
static TIME_TRIGGER: Table<22> = trigger_table!(gs::<{ f::T_RANDOM }>, ps::<{ f::T_RANDOM }>);
static DAILY_TRIGGER: Table<24> = trigger_table!(
    gh::<{ f::T_DAYS_INTERVAL }>,
    ph::<{ f::T_DAYS_INTERVAL }>,
    gs::<{ f::T_RANDOM }>,
    ps::<{ f::T_RANDOM }>,
);
static WEEKLY_TRIGGER: Table<26> = trigger_table!(
    gh::<{ f::T_DAYS_OF_WEEK }>,
    ph::<{ f::T_DAYS_OF_WEEK }>,
    gh::<{ f::T_WEEKS_INTERVAL }>,
    ph::<{ f::T_WEEKS_INTERVAL }>,
    gs::<{ f::T_RANDOM }>,
    ps::<{ f::T_RANDOM }>,
);
static MONTHLY_TRIGGER: Table<28> = trigger_table!(
    gl::<{ f::T_DAYS_OF_MONTH }, 0>,
    pl::<{ f::T_DAYS_OF_MONTH }, 0>,
    gh::<{ f::T_MONTHS }>,
    ph::<{ f::T_MONTHS }>,
    gb::<{ f::T_LAST_DAY }, 0>,
    pb::<{ f::T_LAST_DAY }, 0>,
    gs::<{ f::T_RANDOM }>,
    ps::<{ f::T_RANDOM }>,
);
static MONTHLYDOW_TRIGGER: Table<30> = trigger_table!(
    gh::<{ f::T_DAYS_OF_WEEK }>,
    ph::<{ f::T_DAYS_OF_WEEK }>,
    gh::<{ f::T_WEEKS_OF_MONTH }>,
    ph::<{ f::T_WEEKS_OF_MONTH }>,
    gh::<{ f::T_MONTHS }>,
    ph::<{ f::T_MONTHS }>,
    gb::<{ f::T_LAST_WEEK }, 0>,
    pb::<{ f::T_LAST_WEEK }, 0>,
    gs::<{ f::T_RANDOM }>,
    ps::<{ f::T_RANDOM }>,
);
static BOOT_TRIGGER: Table<22> = trigger_table!(gs::<{ f::T_DELAY }>, ps::<{ f::T_DELAY }>);
static REGISTRATION_TRIGGER: Table<22> = trigger_table!(gs::<{ f::T_DELAY }>, ps::<{ f::T_DELAY }>);
static REPETITION: Table<13> = dispatch1!(
    gs::<{ f::R_INTERVAL }>,
    ps::<{ f::R_INTERVAL }>,
    gs::<{ f::R_DURATION }>,
    ps::<{ f::R_DURATION }>,
    gb::<{ f::R_STOP }, 0>,
    pb::<{ f::R_STOP }, 0>,
);

// ---- actions --------------------------------------------------------------------------------
static ACTIONS: Table<17> = dispatch1!(
    actions_count,
    actions_item,
    actions_enum,
    get_xml,
    put_xml,
    actions_create,
    actions_remove,
    actions_clear,
    gs::<{ f::CONTEXT }>,
    ps::<{ f::CONTEXT }>,
);
extern "system" fn actions_count(this: usize, out: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let out = door_windows_abi::widen(out);
    {
        match the_def(this) {
            Ok(d) => result(write_u32(out, d.parts.with(|p| p.actions.len()) as u32)),
            Err(e) => e,
        }
    }
}
extern "system" fn actions_item(this: usize, index: i32, out: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let out = door_windows_abi::widen(out);
    {
        if out == 0 {
            return hr::E_POINTER;
        }
        let _ = write_pointer(out, 0);
        let Ok(d) = the_def(this) else {
            return hr::E_FAIL;
        };
        if index < 1 {
            return hr::E_INVALIDARG;
        }
        match d.parts.with(|p| p.actions.get(index as usize - 1).cloned()) {
            Some(item) => result(write_pointer(out, action_object(&item))),
            None => hr::E_FAIL,
        }
    }
}
extern "system" fn actions_enum(this: usize, out: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let out = door_windows_abi::widen(out);
    {
        let Ok(d) = the_def(this) else {
            return hr::E_FAIL;
        };
        let items: Vec<u64> = d
            .parts
            .with(|p| p.actions.clone())
            .iter()
            .map(action_object)
            .collect();
        let e = new_enum(items.clone());
        for i in items {
            release(i);
        }
        result(write_pointer(out, e))
    }
}
extern "system" fn actions_create(this: usize, kind: u32, out: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let out = door_windows_abi::widen(out);
    {
        if out == 0 {
            return hr::E_POINTER;
        }
        let _ = write_pointer(out, 0);
        if !matches!(
            kind,
            action::EXEC | action::COM_HANDLER | action::SEND_EMAIL | action::SHOW_MESSAGE
        ) {
            return hr::E_INVALIDARG;
        }
        let Ok(d) = the_def(this) else {
            return hr::E_FAIL;
        };
        let item = Arc::new(Lock::new((Action::new(kind), 0)));
        d.parts.with(|p| p.actions.push(item.clone()));
        result(write_pointer(out, action_object(&item)))
    }
}
extern "system" fn actions_remove(this: usize, which: VariantArg) -> u32 {
    let this = door_windows_abi::widen(this);
    let which = which.at();
    {
        let (Ok(d), Ok(which)) = (the_def(this), variant(which)) else {
            return hr::E_INVALIDARG;
        };
        d.parts
            .with(|p| remove_at(&mut p.actions, which, |a| a.id.as_ref()))
    }
}
extern "system" fn actions_clear(this: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    {
        match the_def(this) {
            Ok(d) => {
                d.parts.with(|p| p.actions.clear());
                hr::S_OK
            }
            Err(e) => e,
        }
    }
}
fn action_of(this: u64) -> Option<&'static Arc<Item<Action>>> {
    match part(this)? {
        Part::Action(i) => Some(i),
        _ => None,
    }
}
extern "system" fn action_type(this: usize, out: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let out = door_windows_abi::widen(out);
    {
        match action_of(this) {
            Some(i) => result(write_u32(out, i.with(|(a, _)| a.kind))),
            None => hr::E_FAIL,
        }
    }
}
extern "system" fn email_fields(this: usize, out: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let out = door_windows_abi::widen(out);
    {
        match action_of(this) {
            Some(i) => out_part(out, Part::ActionPairs(i.clone())),
            None => hr::E_FAIL,
        }
    }
}
extern "system" fn email_put_fields(this: usize, p: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let p = door_windows_abi::widen(p);
    {
        let Some(i) = action_of(this) else {
            return hr::E_FAIL;
        };
        match pairs_of(p) {
            Ok(v) => {
                i.with(|(a, _)| a.header_fields = v);
                hr::S_OK
            }
            Err(e) => e,
        }
    }
}
/// Attachments are not kept (deviation: SendEmail actions do not run).
extern "system" fn email_attachments(_this: usize, out: usize) -> u32 {
    let out = door_windows_abi::widen(out);
    result(write_pointer(out, 0))
}
extern "system" fn email_put_attachments(_this: usize, _array: usize) -> u32 {
    hr::E_NOTIMPL
}
macro_rules! action_table {
    ($($rest:expr),* $(,)?) => {
        dispatch1!(gs::<{ f::A_ID }>, ps::<{ f::A_ID }>, action_type, $($rest),*)
    };
}
static EXEC: Table<18> = action_table!(
    gs::<{ f::A_PATH }>,
    ps::<{ f::A_PATH }>,
    gs::<{ f::A_ARGUMENTS }>,
    ps::<{ f::A_ARGUMENTS }>,
    gs::<{ f::A_DIRECTORY }>,
    ps::<{ f::A_DIRECTORY }>,
    gb::<{ f::A_HIDE }, 0>,
    pb::<{ f::A_HIDE }, 0>,
);
static COM_HANDLER: Table<14> = action_table!(
    gs::<{ f::A_CLASS }>,
    ps::<{ f::A_CLASS }>,
    gs::<{ f::A_DATA }>,
    ps::<{ f::A_DATA }>,
);
static EMAIL: Table<30> = action_table!(
    gs::<{ f::A_SERVER }>,
    ps::<{ f::A_SERVER }>,
    gs::<{ f::A_SUBJECT }>,
    ps::<{ f::A_SUBJECT }>,
    gs::<{ f::A_TO }>,
    ps::<{ f::A_TO }>,
    gs::<{ f::A_CC }>,
    ps::<{ f::A_CC }>,
    gs::<{ f::A_BCC }>,
    ps::<{ f::A_BCC }>,
    gs::<{ f::A_REPLY_TO }>,
    ps::<{ f::A_REPLY_TO }>,
    gs::<{ f::A_FROM }>,
    ps::<{ f::A_FROM }>,
    email_fields,
    email_put_fields,
    gs::<{ f::A_BODY }>,
    ps::<{ f::A_BODY }>,
    email_attachments,
    email_put_attachments,
);
static SHOW_MESSAGE: Table<14> = action_table!(
    gs::<{ f::A_TITLE }>,
    ps::<{ f::A_TITLE }>,
    gs::<{ f::A_BODY }>,
    ps::<{ f::A_BODY }>,
);

// ---- settings and principal --------------------------------------------------------------
static SETTINGS: Table<56> = dispatch1!(
    gb::<{ f::S_DEMAND }, 0>,
    pb::<{ f::S_DEMAND }, 0>,
    gs::<{ f::S_RESTART }>,
    ps::<{ f::S_RESTART }>,
    gl::<{ f::S_RESTART_COUNT }, 0>,
    pl::<{ f::S_RESTART_COUNT }, 0>,
    gl::<{ f::S_INSTANCES }, 0>,
    pl::<{ f::S_INSTANCES }, 0>,
    gb::<{ f::S_STOP_BATTERIES }, 0>,
    pb::<{ f::S_STOP_BATTERIES }, 0>,
    gb::<{ f::S_DISALLOW_BATTERIES }, 0>,
    pb::<{ f::S_DISALLOW_BATTERIES }, 0>,
    gb::<{ f::S_HARD }, 0>,
    pb::<{ f::S_HARD }, 0>,
    gb::<{ f::S_WHEN_AVAILABLE }, 0>,
    pb::<{ f::S_WHEN_AVAILABLE }, 0>,
    get_xml,
    put_xml,
    gb::<{ f::S_NETWORK }, 0>,
    pb::<{ f::S_NETWORK }, 0>,
    gs::<{ f::S_LIMIT }>,
    ps::<{ f::S_LIMIT }>,
    gb::<{ f::S_ENABLED }, 0>,
    pb::<{ f::S_ENABLED }, 0>,
    gs::<{ f::S_DELETE }>,
    ps::<{ f::S_DELETE }>,
    gl::<{ f::S_PRIORITY }, 0>,
    pl::<{ f::S_PRIORITY }, 0>,
    gl::<{ f::S_COMPATIBILITY }, 0>,
    pl::<{ f::S_COMPATIBILITY }, 0>,
    gb::<{ f::S_HIDDEN }, 0>,
    pb::<{ f::S_HIDDEN }, 0>,
    settings_idle,
    settings_put_idle,
    gb::<{ f::S_IDLE_ONLY }, 0>,
    pb::<{ f::S_IDLE_ONLY }, 0>,
    gb::<{ f::S_WAKE }, 0>,
    pb::<{ f::S_WAKE }, 0>,
    settings_network,
    settings_put_network,
    gb::<{ f::S_REMOTE_APP }, 0>,
    pb::<{ f::S_REMOTE_APP }, 0>,
    gb::<{ f::S_UNIFIED }, 0>,
    pb::<{ f::S_UNIFIED }, 0>,
    settings_maintenance,
    settings_put_maintenance,
    settings_create_maintenance,
    gb::<{ f::S_VOLATILE }, 0>,
    pb::<{ f::S_VOLATILE }, 0>,
);
static SETTINGS2: Table<11> = dispatch2!(
    gb::<{ f::S_REMOTE_APP }, 8>,
    pb::<{ f::S_REMOTE_APP }, 8>,
    gb::<{ f::S_UNIFIED }, 8>,
    pb::<{ f::S_UNIFIED }, 8>,
);
extern "system" fn settings_idle(this: usize, out: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let out = door_windows_abi::widen(out);
    {
        match the_def(this) {
            Ok(d) => out_part(out, Part::Idle(d.clone())),
            Err(e) => e,
        }
    }
}
extern "system" fn settings_put_idle(this: usize, p: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let p = door_windows_abi::widen(p);
    {
        let (d, o) = match (the_def(this), other(p)) {
            (Ok(d), Ok(o)) => (d, o),
            (Err(e), _) | (_, Err(e)) => return e,
        };
        let idle = o.parts.with(|x| x.settings.idle.clone());
        d.parts.with(|x| x.settings.idle = idle);
        hr::S_OK
    }
}
extern "system" fn settings_network(this: usize, out: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let out = door_windows_abi::widen(out);
    {
        match the_def(this) {
            Ok(d) => out_part(out, Part::Network(d.clone())),
            Err(e) => e,
        }
    }
}
extern "system" fn settings_put_network(this: usize, p: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let p = door_windows_abi::widen(p);
    {
        let (d, o) = match (the_def(this), other(p)) {
            (Ok(d), Ok(o)) => (d, o),
            (Err(e), _) | (_, Err(e)) => return e,
        };
        let n = o.parts.with(|x| {
            (
                x.settings.network_name.clone(),
                x.settings.network_id.clone(),
            )
        });
        d.parts
            .with(|x| (x.settings.network_name, x.settings.network_id) = n);
        hr::S_OK
    }
}
extern "system" fn settings_maintenance(this: usize, out: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let out = door_windows_abi::widen(out);
    {
        if out == 0 {
            return hr::E_POINTER;
        }
        match the_def(this) {
            Ok(d) if d.parts.with(|p| p.maintenance.is_some()) => {
                out_part(out, Part::Maintenance(d.clone()))
            }
            Ok(_) => result(write_pointer(out, 0)),
            Err(e) => e,
        }
    }
}
extern "system" fn settings_put_maintenance(this: usize, p: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let p = door_windows_abi::widen(p);
    {
        let Ok(d) = the_def(this) else {
            return hr::E_FAIL;
        };
        let m = if p == 0 {
            None
        } else {
            match other(p) {
                Ok(o) => o.parts.with(|x| x.maintenance.clone()),
                Err(e) => return e,
            }
        };
        d.parts.with(|x| x.maintenance = m);
        hr::S_OK
    }
}
extern "system" fn settings_create_maintenance(this: usize, out: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let out = door_windows_abi::widen(out);
    {
        match the_def(this) {
            Ok(d) => {
                d.parts.with(|p| p.maintenance = Some((None, None, false)));
                out_part(out, Part::Maintenance(d.clone()))
            }
            Err(e) => e,
        }
    }
}
static IDLE: Table<15> = dispatch1!(
    gs::<{ f::I_DURATION }>,
    ps::<{ f::I_DURATION }>,
    gs::<{ f::I_WAIT }>,
    ps::<{ f::I_WAIT }>,
    gb::<{ f::I_STOP }, 0>,
    pb::<{ f::I_STOP }, 0>,
    gb::<{ f::I_RESTART }, 0>,
    pb::<{ f::I_RESTART }, 0>,
);
static NETWORK: Table<11> = dispatch1!(
    gs::<{ f::N_NAME }>,
    ps::<{ f::N_NAME }>,
    gs::<{ f::N_ID }>,
    ps::<{ f::N_ID }>,
);
static MAINTENANCE: Table<13> = dispatch1!(
    ps::<{ f::M_PERIOD }>,
    gs::<{ f::M_PERIOD }>,
    ps::<{ f::M_DEADLINE }>,
    gs::<{ f::M_DEADLINE }>,
    pb::<{ f::M_EXCLUSIVE }, 0>,
    gb::<{ f::M_EXCLUSIVE }, 0>,
);
static PRINCIPAL: Table<19> = dispatch1!(
    gs::<{ f::P_ID }>,
    ps::<{ f::P_ID }>,
    gs::<{ f::P_DISPLAY }>,
    ps::<{ f::P_DISPLAY }>,
    gs::<{ f::P_USER }>,
    ps::<{ f::P_USER }>,
    gl::<{ f::P_LOGON }, 0>,
    pl::<{ f::P_LOGON }, 0>,
    gs::<{ f::P_GROUP }>,
    ps::<{ f::P_GROUP }>,
    gl::<{ f::P_RUN_LEVEL }, 0>,
    pl::<{ f::P_RUN_LEVEL }, 0>,
);
static PRINCIPAL2: Table<12> = dispatch2!(
    gl::<{ f::P_SID_TYPE }, 8>,
    pl::<{ f::P_SID_TYPE }, 8>,
    privileges_count,
    privileges_item,
    privileges_add,
);
extern "system" fn privileges_count(this: usize, out: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let out = door_windows_abi::widen(out);
    {
        match the_def(this - 8) {
            Ok(d) => result(write_u32(out, d.parts.with(|p| p.privileges.len()) as u32)),
            Err(e) => e,
        }
    }
}
extern "system" fn privileges_item(this: usize, index: i32, out: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let out = door_windows_abi::widen(out);
    {
        let Ok(d) = the_def(this - 8) else {
            return hr::E_FAIL;
        };
        match d
            .parts
            .with(|p| p.privileges.get((index - 1).max(0) as usize).cloned())
        {
            Some(n) if index >= 1 => bstr_out(out, Some(&n)),
            _ => hr::E_INVALIDARG,
        }
    }
}
extern "system" fn privileges_add(this: usize, name: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let name = door_windows_abi::widen(name);
    {
        let (Ok(d), Some(name)) = (the_def(this - 8), bstr_in(name)) else {
            return hr::E_INVALIDARG;
        };
        d.parts.with(|p| p.privileges.push(name));
        hr::S_OK
    }
}

// ---- named values -------------------------------------------------------------------------
static PAIRS: Table<13> = dispatch1!(
    pairs_count,
    pairs_item,
    pairs_enum,
    pairs_create,
    pairs_remove,
    pairs_clear,
);
static PAIR: Table<11> = dispatch1!(
    gs::<{ f::PAIR_NAME }>,
    ps::<{ f::PAIR_NAME }>,
    gs::<{ f::PAIR_VALUE }>,
    ps::<{ f::PAIR_VALUE }>,
);
/// Runs `f` on the trigger's value queries or the action's header fields.
fn with_pairs<R>(this: u64, f: impl FnOnce(&mut Vec<(String, String)>) -> R) -> Result<R, u32> {
    match part(this) {
        Some(Part::TriggerPairs(i)) => Ok(i.with(|(t, _)| f(&mut t.value_queries))),
        Some(Part::ActionPairs(i)) => Ok(i.with(|(a, _)| f(&mut a.header_fields))),
        _ => Err(hr::E_FAIL),
    }
}
fn pairs_of(p: u64) -> Result<Vec<(String, String)>, u32> {
    if !ours(p) {
        return Err(hr::E_INVALIDARG);
    }
    with_pairs(p, |v| v.clone()).map_err(|_| hr::E_INVALIDARG)
}
extern "system" fn pairs_count(this: usize, out: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let out = door_windows_abi::widen(out);
    {
        match with_pairs(this, |v| v.len()) {
            Ok(n) => result(write_u32(out, n as u32)),
            Err(e) => e,
        }
    }
}
/// A pair object holds a copy (a pair changed through it changes only it:
/// deviation from Windows, whose pair objects are live).
fn pair_object(name: String, value: String) -> u64 {
    make(Part::Pair(Arc::new(Lock::new((name, value)))))
}
extern "system" fn pairs_item(this: usize, index: i32, out: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let out = door_windows_abi::widen(out);
    {
        if out == 0 {
            return hr::E_POINTER;
        }
        match with_pairs(this, |v| v.get((index - 1).max(0) as usize).cloned()) {
            Ok(Some((n, v))) if index >= 1 => result(write_pointer(out, pair_object(n, v))),
            Ok(_) => hr::E_INVALIDARG,
            Err(e) => e,
        }
    }
}
extern "system" fn pairs_enum(this: usize, out: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let out = door_windows_abi::widen(out);
    {
        let items: Vec<u64> = match with_pairs(this, |v| v.clone()) {
            Ok(v) => v.into_iter().map(|(n, v)| pair_object(n, v)).collect(),
            Err(e) => return e,
        };
        let e = new_enum(items.clone());
        for i in items {
            release(i);
        }
        result(write_pointer(out, e))
    }
}
extern "system" fn pairs_create(this: usize, name: usize, value: usize, out: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let name = door_windows_abi::widen(name);
    let value = door_windows_abi::widen(value);
    let out = door_windows_abi::widen(out);
    {
        let n = bstr_in(name).unwrap_or_default();
        let v = bstr_in(value).unwrap_or_default();
        if let Err(e) = with_pairs(this, |list| list.push((n.clone(), v.clone()))) {
            return e;
        }
        if out == 0 {
            return hr::S_OK;
        }
        result(write_pointer(out, pair_object(n, v)))
    }
}
extern "system" fn pairs_remove(this: usize, index: i32) -> u32 {
    let this = door_windows_abi::widen(this);
    {
        match with_pairs(this, |list| {
            if index >= 1 && (index as usize) <= list.len() {
                list.remove(index as usize - 1);
                true
            } else {
                false
            }
        }) {
            Ok(true) => hr::S_OK,
            Ok(false) => hr::E_INVALIDARG,
            Err(e) => e,
        }
    }
}
extern "system" fn pairs_clear(this: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    result(with_pairs(this, |list| list.clear()))
}
