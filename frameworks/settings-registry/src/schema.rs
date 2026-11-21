//! Property declarations: types, storage words, scope, permissions,
//! dependencies, apply timing and the static schema check.
use door_service_client::{RegistryRoot, Setting};
use door_settings_map::Page;

/// Most properties a schema may declare ([`PropertySet`] is a 128-bit set).
pub const MAX_PROPERTIES: usize = 128;
/// Most storage words a schema may declare.
pub const MAX_WORDS: usize = 16;

/// A property's index in its schema.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PropertyId(pub(crate) u8);
impl PropertyId {
    pub const fn index(self) -> usize {
        self.0 as usize
    }
}

/// A set of properties (fixed capacity, allocation-free).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PropertySet(u128);
impl PropertySet {
    pub const EMPTY: Self = Self(0);
    /// Every property of any schema.
    pub const ALL: Self = Self(u128::MAX);
    pub const fn of(id: PropertyId) -> Self {
        Self(1 << id.0)
    }
    pub const fn contains(self, id: PropertyId) -> bool {
        self.0 & (1 << id.0) != 0
    }
    pub fn insert(&mut self, id: PropertyId) {
        self.0 |= 1 << id.0;
    }
    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }
    pub const fn intersection(self, other: Self) -> Self {
        Self(self.0 & other.0)
    }
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }
    pub const fn len(self) -> usize {
        self.0.count_ones() as usize
    }
    /// Members in ascending id order.
    pub fn iter(self) -> impl Iterator<Item = PropertyId> {
        (0..MAX_PROPERTIES as u8)
            .filter(move |i| self.0 & (1 << i) != 0)
            .map(PropertyId)
    }
}

/// Who owns a value: the signed-in user (HKCU and per-user storage words)
/// or the machine (HKLM).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scope {
    User,
    Machine,
}

/// Authority needed to change a value. `Administrator` is the existing
/// privileged path: HKLM is writable only by system authority today, so an
/// ordinary Settings process reads such values and cannot change them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Authority {
    User,
    Administrator,
}

/// The authority a registry instance (or a search) acts with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Caller {
    /// An ordinary signed-in user process (Settings).
    User,
    /// A process holding system authority (a privileged broker).
    Administrator,
}
impl Caller {
    pub const fn may(self, authority: Authority) -> bool {
        matches!(
            (self, authority),
            (_, Authority::User) | (Caller::Administrator, Authority::Administrator)
        )
    }
}

/// Search listing policy.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Visibility {
    /// Listed for every caller that can read it (read-only when it cannot
    /// change it).
    Public,
    /// Listed only for callers that may change it.
    Private,
    /// Never listed (encodings such as pin order ranks).
    Internal,
}

/// When a committed value takes effect.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Default)]
pub enum Apply {
    #[default]
    Live,
    SignOut,
    Restart,
}

/// Named choices of a catalog maintained elsewhere (time zones, regional
/// formats): `name` answers for valid ids only; `nth` lists ids in display
/// order.
#[derive(Clone, Copy, Debug)]
pub struct Catalog {
    pub name: fn(u32) -> Option<&'static str>,
    pub nth: fn(usize) -> Option<u32>,
}

/// A property's value type. Every value is carried as a `u32`: 0/1 for
/// `Bool`, the stored number for the others, exactly as persisted.
#[derive(Clone, Copy, Debug)]
pub enum Kind {
    Bool,
    /// One of these (value, name) pairs.
    Choice(&'static [(u32, &'static str)]),
    /// An id of an external catalog.
    Catalog(&'static Catalog),
    /// `min..=max` in steps of `step` from `min`, plus an optional special
    /// value outside the range (0 = "Automatic" for the display scale).
    Integer {
        min: u32,
        max: u32,
        step: u32,
        special: Option<(u32, &'static str)>,
    },
    /// Bit `i` is the flag `names[i]`; no other bits.
    Bitset(&'static [&'static str]),
    /// An encoded structure validated only by its storage word's validator
    /// (the input method list).
    Encoded,
}
impl Kind {
    /// Whether `value` has this type (the storage word may still refuse
    /// the combination).
    pub fn accepts(self, value: u32) -> bool {
        match self {
            Kind::Bool => value <= 1,
            Kind::Choice(options) => options.iter().any(|(v, _)| *v == value),
            Kind::Catalog(catalog) => (catalog.name)(value).is_some(),
            Kind::Integer {
                min,
                max,
                step,
                special,
            } => {
                special.is_some_and(|(v, _)| v == value)
                    || (value >= min
                        && value <= max
                        && step != 0
                        && (value - min).is_multiple_of(step))
            }
            Kind::Bitset(names) => names.len() >= 32 || value >> names.len() == 0,
            Kind::Encoded => true,
        }
    }
    /// Display name of `value` (choices, catalogs, the special integer).
    pub fn label(self, value: u32) -> Option<&'static str> {
        match self {
            Kind::Bool => Some(if value != 0 { "On" } else { "Off" }),
            Kind::Choice(options) => options.iter().find(|(v, _)| *v == value).map(|o| o.1),
            Kind::Catalog(catalog) => (catalog.name)(value),
            Kind::Integer { special, .. } => special.filter(|(v, _)| *v == value).map(|s| s.1),
            Kind::Bitset(_) | Kind::Encoded => None,
        }
    }
}

/// Where a storage word lives.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Store {
    /// A storage-service `Setting` (per user, validated by storage-core).
    Setting(Setting),
    /// A REG_DWORD value under HKLM or HKCU (64-bit view).
    Registry {
        root: RegistryRoot,
        path: &'static str,
        name: &'static str,
    },
}

/// Another word the storage service changes when this one is written
/// (`Theme` and `DesktopAppearance` share the dark bit). `map(written,
/// other_before)` is the other word afterwards.
#[derive(Clone, Copy, Debug)]
pub struct Coupling {
    pub word: u8,
    pub map: fn(u32, u32) -> u32,
}

/// One persisted 32-bit value holding one or more properties.
#[derive(Clone, Copy, Debug)]
pub struct Word {
    pub store: Store,
    pub default: u32,
    /// The storage-side validation, unchanged.
    pub valid: fn(u32) -> bool,
    pub scope: Scope,
    pub write: Authority,
    pub couple: Option<Coupling>,
}

/// A rule relating a property to another one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dependency {
    /// Takes effect only while `key` holds `value` (shown disabled
    /// otherwise; the stored value is kept).
    ActiveWhen { key: &'static str, value: u32 },
    /// While `key` does not hold `value`, the effective value is `forced`.
    ForcedUnless {
        key: &'static str,
        value: u32,
        forced: u32,
    },
}

/// A typed property: one bit field of one storage word.
#[derive(Clone, Copy, Debug)]
pub struct Property {
    /// Stable key (`area.name`, lower case); never reused.
    pub key: &'static str,
    /// Visible title (search matches it).
    pub title: &'static str,
    /// Extra search words.
    pub keywords: &'static str,
    pub kind: Kind,
    pub default: u32,
    /// Index of the storage word in the schema.
    pub word: u8,
    /// Bit field `shift..shift + width` of the word.
    pub shift: u8,
    pub width: u8,
    pub apply: Apply,
    pub page: Page,
    /// `door_settings_map` section anchor on `page`.
    pub anchor: Option<&'static str>,
    pub visibility: Visibility,
    pub depends: &'static [Dependency],
}
impl Property {
    pub const fn mask(&self) -> u32 {
        if self.width >= 32 {
            u32::MAX
        } else {
            ((1u32 << self.width) - 1) << self.shift
        }
    }
    pub const fn extract(&self, word: u32) -> u32 {
        (word & self.mask()) >> self.shift
    }
    /// `word` with this field set to `value` (`None` if it does not fit).
    pub const fn insert(&self, word: u32, value: u32) -> Option<u32> {
        if self.width < 32 && value >> self.width != 0 {
            return None;
        }
        Some((word & !self.mask()) | (value << self.shift))
    }
}

/// A complete property table.
#[derive(Clone, Copy, Debug)]
pub struct Schema {
    pub words: &'static [Word],
    pub properties: &'static [Property],
}

/// Why a schema was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SchemaError {
    TooManyProperties,
    TooManyWords,
    /// A word's default fails its own validator.
    WordDefault(usize),
    /// A coupling names a missing word or itself.
    Coupling(usize),
    /// Empty key or a character outside `a-z 0-9 . _`.
    KeySyntax(&'static str),
    DuplicateKey(&'static str),
    /// Missing word, empty field or a field past bit 31.
    Field(&'static str),
    /// Two properties share bits of one word.
    Overlap(&'static str),
    /// Empty choice list, zero step, inverted range or a type wider than
    /// its field.
    Kind(&'static str),
    /// The default is not a value of the type or disagrees with the word
    /// default's bits.
    Default(&'static str),
    /// A dependency names an unknown key, the property itself, or a value
    /// the controlling property cannot hold.
    Dependency(&'static str),
    /// The anchor is not a section of the page.
    Anchor(&'static str),
}

pub(crate) fn find(properties: &[Property], key: &str) -> Option<usize> {
    properties.iter().position(|p| p.key == key)
}

/// Check a schema: every rule a registry relies on.
pub fn validate(schema: &Schema) -> Result<(), SchemaError> {
    let (words, properties) = (schema.words, schema.properties);
    if properties.len() > MAX_PROPERTIES {
        return Err(SchemaError::TooManyProperties);
    }
    if words.len() > MAX_WORDS {
        return Err(SchemaError::TooManyWords);
    }
    for (index, word) in words.iter().enumerate() {
        if !(word.valid)(word.default) {
            return Err(SchemaError::WordDefault(index));
        }
        if let Some(couple) = word.couple
            && (usize::from(couple.word) >= words.len() || usize::from(couple.word) == index)
        {
            return Err(SchemaError::Coupling(index));
        }
    }
    for (index, p) in properties.iter().enumerate() {
        if p.key.is_empty()
            || !p
                .key
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'.' || b == b'_')
        {
            return Err(SchemaError::KeySyntax(p.key));
        }
        if properties[..index].iter().any(|q| q.key == p.key) {
            return Err(SchemaError::DuplicateKey(p.key));
        }
        let Some(word) = words.get(usize::from(p.word)) else {
            return Err(SchemaError::Field(p.key));
        };
        if p.width == 0 || u32::from(p.shift) + u32::from(p.width) > 32 {
            return Err(SchemaError::Field(p.key));
        }
        if properties[..index]
            .iter()
            .any(|q| q.word == p.word && q.mask() & p.mask() != 0)
        {
            return Err(SchemaError::Overlap(p.key));
        }
        let fits = |value: u32| p.width >= 32 || value >> p.width == 0;
        let kind_ok = match p.kind {
            Kind::Bool => true,
            Kind::Choice(options) => !options.is_empty() && options.iter().all(|o| fits(o.0)),
            Kind::Catalog(catalog) => (catalog.nth)(0).is_some(),
            Kind::Integer {
                min,
                max,
                step,
                special,
            } => step != 0 && min <= max && fits(max) && special.is_none_or(|s| fits(s.0)),
            Kind::Bitset(names) => !names.is_empty() && names.len() <= usize::from(p.width),
            Kind::Encoded => true,
        };
        if !kind_ok {
            return Err(SchemaError::Kind(p.key));
        }
        if !p.kind.accepts(p.default) || p.extract(word.default) != p.default {
            return Err(SchemaError::Default(p.key));
        }
        for dependency in p.depends {
            let (key, value) = match *dependency {
                Dependency::ActiveWhen { key, value } => (key, value),
                Dependency::ForcedUnless { key, value, forced } => {
                    if !p.kind.accepts(forced) {
                        return Err(SchemaError::Dependency(p.key));
                    }
                    (key, value)
                }
            };
            match find(properties, key) {
                Some(other) if other != index && properties[other].kind.accepts(value) => {}
                _ => return Err(SchemaError::Dependency(p.key)),
            }
        }
        if let Some(anchor) = p.anchor
            && door_settings_map::section(p.page, anchor).is_none()
        {
            return Err(SchemaError::Anchor(p.key));
        }
    }
    Ok(())
}
