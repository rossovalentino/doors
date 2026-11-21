//! Semantic node vocabulary: roles, states, actions and geometry. The roles
//! follow UI Automation control types (Narrator speaks their localized names);
//! states and actions follow the UIA patterns a control supports.
use alloc::{string::String, vec::Vec};

/// Stable, opaque, nonzero node identity within one window's tree. Toolkits
/// derive it from generational widget identity, never from addresses, so a
/// recycled widget slot never aliases an earlier node.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NodeId(pub u64);
impl NodeId {
    pub const fn new(raw: u64) -> Option<Self> {
        if raw == 0 { None } else { Some(Self(raw)) }
    }
    pub const fn raw(self) -> u64 {
        self.0
    }
}

macro_rules! roles {
    ($($name:ident = $value:literal, $spoken:literal;)*) => {
        /// UI Automation-style control type (wire value in brackets).
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        #[repr(u16)]
        pub enum Role { $($name = $value,)* }
        impl Role {
            pub const ALL: &'static [Role] = &[$(Role::$name,)*];
            pub const fn from_wire(value: u16) -> Option<Self> {
                match value { $($value => Some(Self::$name),)* _ => None }
            }
            pub const fn wire(self) -> u16 { self as u16 }
            /// The control type name a screen reader speaks (Narrator, en-US).
            pub const fn spoken(self) -> &'static str {
                match self { $(Self::$name => $spoken,)* }
            }
        }
    };
}
roles! {
    Window = 1, "window";
    Pane = 2, "pane";
    Group = 3, "group";
    Text = 4, "text";
    Button = 5, "button";
    ToggleButton = 6, "toggle button";
    CheckBox = 7, "check box";
    RadioButton = 8, "radio button";
    ToggleSwitch = 9, "toggle switch";
    Slider = 10, "slider";
    ComboBox = 11, "combo box";
    Edit = 12, "edit";
    Spinner = 13, "spinner";
    List = 14, "list";
    ListItem = 15, "list item";
    Tree = 16, "tree";
    TreeItem = 17, "tree item";
    TabList = 18, "tab";
    Tab = 19, "tab item";
    TabPanel = 20, "pane";
    MenuBar = 21, "menu bar";
    Menu = 22, "menu";
    MenuItem = 23, "menu item";
    Dialog = 24, "dialog";
    ToolTip = 25, "tool tip";
    InfoBar = 26, "info bar";
    ProgressBar = 27, "progress bar";
    ScrollViewer = 28, "scroll viewer";
    ScrollBar = 29, "scroll bar";
    Navigation = 30, "navigation";
    Expander = 31, "expander";
    ToolBar = 32, "tool bar";
    Hyperlink = 33, "link";
    Image = 34, "image";
    Separator = 35, "separator";
    Heading = 36, "heading";
    Document = 37, "document";
    Status = 38, "status";
}

macro_rules! flags {
    ($(#[$doc:meta])* $ty:ident { $($(#[$fdoc:meta])* $name:ident = $bit:literal,)* }) => {
        $(#[$doc])*
        #[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
        pub struct $ty(pub u32);
        impl $ty {
            $($(#[$fdoc])* pub const $name: Self = Self(1 << $bit);)*
            pub const NONE: Self = Self(0);
            pub const KNOWN: u32 = 0 $(| (1 << $bit))*;
            pub const fn bits(self) -> u32 { self.0 }
            pub const fn from_bits(bits: u32) -> Option<Self> {
                if bits & !Self::KNOWN != 0 { None } else { Some(Self(bits)) }
            }
            pub const fn contains(self, other: Self) -> bool { self.0 & other.0 == other.0 }
            pub const fn intersects(self, other: Self) -> bool { self.0 & other.0 != 0 }
            pub const fn with(self, other: Self, on: bool) -> Self {
                if on { Self(self.0 | other.0) } else { Self(self.0 & !other.0) }
            }
            pub fn set(&mut self, other: Self, on: bool) { *self = self.with(other, on); }
            /// Names of the set flags, for stored semantic-tree fixtures.
            pub fn names(self) -> impl Iterator<Item = &'static str> {
                [$((Self::$name, stringify!($name)),)*]
                    .into_iter()
                    .filter(move |(f, _)| self.contains(*f))
                    .map(|(_, n)| n)
            }
        }
        impl core::ops::BitOr for $ty {
            type Output = Self;
            fn bitor(self, o: Self) -> Self { Self(self.0 | o.0) }
        }
        impl core::ops::BitOrAssign for $ty {
            fn bitor_assign(&mut self, o: Self) { self.0 |= o.0; }
        }
    };
}
flags!(
    /// Dynamic state of a node (UIA properties and pattern state).
    States {
        FOCUSABLE = 0,
        FOCUSED = 1,
        DISABLED = 2,
        /// Toggle state On (check box, toggle switch, toggle button).
        CHECKED = 3,
        /// Toggle state Indeterminate.
        MIXED = 4,
        SELECTED = 5,
        /// Supports ExpandCollapse; EXPANDED tells which.
        EXPANDABLE = 6,
        EXPANDED = 7,
        PRESSED = 8,
        READ_ONLY = 9,
        REQUIRED = 10,
        INVALID = 11,
        /// Password content: value and text ranges are never published.
        PROTECTED = 12,
        BUSY = 13,
        OFFSCREEN = 14,
        MULTILINE = 15,
        MODAL = 16,
        SELECTABLE = 17,
        MULTISELECTABLE = 18,
        /// Supports the Toggle pattern (CHECKED/MIXED meaningful).
        CHECKABLE = 19,
        /// The default button of a dialog.
        DEFAULT = 20,
        HORIZONTAL = 21,
        VERTICAL = 22,
        HAS_POPUP = 23,
        /// The value was cut at `wire::MAX_VALUE_BYTES`.
        VALUE_TRUNCATED = 24,
        /// Live region politeness (InfoBar, status text).
        LIVE_POLITE = 25,
        LIVE_ASSERTIVE = 26,
        /// `Node::range` is present (RangeValue pattern).
        HAS_RANGE = 27,
        /// `Node::selection` is present (text selection, UTF-8 byte offsets).
        HAS_SELECTION = 28,
        EDITABLE = 29,
    }
);
flags!(
    /// Actions a node accepts from an authorized assistive client.
    Actions {
        FOCUS = 0,
        /// Invoke pattern (button click, menu item, hyperlink).
        INVOKE = 1,
        TOGGLE = 2,
        EXPAND = 3,
        COLLAPSE = 4,
        /// SelectionItem.Select.
        SELECT = 5,
        /// Value.SetValue (text argument).
        SET_VALUE = 6,
        INCREMENT = 7,
        DECREMENT = 8,
        SCROLL_INTO_VIEW = 9,
        /// Open the context menu (Shift+F10).
        SHOW_MENU = 10,
        /// Close a dialog, flyout, info bar or tool tip.
        DISMISS = 11,
        /// RangeValue.SetValue (number argument, milli-units).
        SET_RANGE = 12,
        /// TextPattern selection (range argument, UTF-8 byte offsets).
        SET_SELECTION = 13,
    }
);

/// One requested action. Arguments travel only with the actions that use them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    Focus,
    Invoke,
    Toggle,
    Expand,
    Collapse,
    Select,
    SetValue(String),
    Increment,
    Decrement,
    ScrollIntoView,
    ShowMenu,
    Dismiss,
    SetRange(i64),
    SetSelection(u32, u32),
}
impl Action {
    pub const fn flag(&self) -> Actions {
        match self {
            Self::Focus => Actions::FOCUS,
            Self::Invoke => Actions::INVOKE,
            Self::Toggle => Actions::TOGGLE,
            Self::Expand => Actions::EXPAND,
            Self::Collapse => Actions::COLLAPSE,
            Self::Select => Actions::SELECT,
            Self::SetValue(_) => Actions::SET_VALUE,
            Self::Increment => Actions::INCREMENT,
            Self::Decrement => Actions::DECREMENT,
            Self::ScrollIntoView => Actions::SCROLL_INTO_VIEW,
            Self::ShowMenu => Actions::SHOW_MENU,
            Self::Dismiss => Actions::DISMISS,
            Self::SetRange(_) => Actions::SET_RANGE,
            Self::SetSelection(..) => Actions::SET_SELECTION,
        }
    }
    /// Wire code: the bit index of `flag()`.
    pub const fn code(&self) -> u32 {
        self.flag().0.trailing_zeros()
    }
    /// Decode a brokered request (`door.accessibility` INVOKE): SET_VALUE
    /// carries UTF-8 text, SET_RANGE an i64 (milli-units), SET_SELECTION two
    /// u32 offsets; other actions carry nothing.
    pub fn from_code(code: u32, arg: &[u8]) -> Option<Self> {
        let none = arg.is_empty();
        Some(match code {
            0 if none => Self::Focus,
            1 if none => Self::Invoke,
            2 if none => Self::Toggle,
            3 if none => Self::Expand,
            4 if none => Self::Collapse,
            5 if none => Self::Select,
            6 => Self::SetValue(String::from(core::str::from_utf8(arg).ok()?)),
            7 if none => Self::Increment,
            8 if none => Self::Decrement,
            9 if none => Self::ScrollIntoView,
            10 if none => Self::ShowMenu,
            11 if none => Self::Dismiss,
            12 => Self::SetRange(i64::from_le_bytes(arg.try_into().ok()?)),
            13 if arg.len() == 8 => Self::SetSelection(
                u32::from_le_bytes(arg[..4].try_into().ok()?),
                u32::from_le_bytes(arg[4..].try_into().ok()?),
            ),
            _ => return None,
        })
    }
    /// The argument bytes `from_code` expects.
    pub fn arg(&self) -> alloc::vec::Vec<u8> {
        match self {
            Self::SetValue(s) => s.as_bytes().to_vec(),
            Self::SetRange(v) => v.to_le_bytes().to_vec(),
            Self::SetSelection(a, c) => [a.to_le_bytes(), c.to_le_bytes()].concat(),
            _ => alloc::vec::Vec::new(),
        }
    }
}

/// Client-local bounds in 26.6 fixed-point logical units (device-independent
/// pixels times 64). Width and height are never negative.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Bounds {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}
impl Bounds {
    pub const fn from_logical(x: i32, y: i32, width: i32, height: i32) -> Self {
        Self {
            x: x.saturating_mul(64),
            y: y.saturating_mul(64),
            width: width.saturating_mul(64),
            height: height.saturating_mul(64),
        }
    }
    pub const fn valid(self) -> bool {
        self.width >= 0
            && self.height >= 0
            && self.x.checked_add(self.width).is_some()
            && self.y.checked_add(self.height).is_some()
    }
    /// Whole logical pixels (rounded to nearest), for fixtures and captions.
    pub const fn logical(self) -> (i32, i32, i32, i32) {
        const fn r(v: i32) -> i32 {
            (v + 32).div_euclid(64)
        }
        (r(self.x), r(self.y), r(self.width), r(self.height))
    }
}

/// RangeValue in milli-units (value times 1000), so integral sliders and
/// fractional number boxes share one exact integer representation.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct RangeValue {
    pub min: i64,
    pub max: i64,
    pub now: i64,
    pub step: i64,
}
impl RangeValue {
    pub const fn valid(self) -> bool {
        self.min <= self.max && self.now >= self.min && self.now <= self.max && self.step >= 0
    }
    pub const fn integral(min: i64, max: i64, now: i64, step: i64) -> Self {
        Self {
            min: min.saturating_mul(1000),
            max: max.saturating_mul(1000),
            now: now.saturating_mul(1000),
            step: step.saturating_mul(1000),
        }
    }
}

/// One semantic node. `children` lists the node's children in reading order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Node {
    pub id: NodeId,
    pub parent: Option<NodeId>,
    pub children: Vec<NodeId>,
    pub role: Role,
    pub name: String,
    /// Value pattern text (edit content, combo box selection, slider text).
    /// Always empty for PROTECTED nodes on the wire.
    pub value: String,
    /// Help text / full description (tool tip content).
    pub description: String,
    /// Accelerator or access key, e.g. "Ctrl+S".
    pub accelerator: String,
    pub states: States,
    pub actions: Actions,
    pub bounds: Bounds,
    pub range: Option<RangeValue>,
    /// (position in set, set size), 1-based.
    pub position: Option<(u32, u32)>,
    /// Hierarchy level (tree items, headings); 0 when not applicable.
    pub level: u16,
    /// Text selection as UTF-8 byte offsets into `value` (anchor, caret).
    pub selection: Option<(u32, u32)>,
    pub labelled_by: Option<NodeId>,
}
impl Node {
    pub fn new(id: NodeId, role: Role) -> Self {
        Self {
            id,
            parent: None,
            children: Vec::new(),
            role,
            name: String::new(),
            value: String::new(),
            description: String::new(),
            accelerator: String::new(),
            states: States::NONE,
            actions: Actions::NONE,
            bounds: Bounds::default(),
            range: None,
            position: None,
            level: 0,
            selection: None,
            labelled_by: None,
        }
    }
    pub fn named(mut self, name: &str) -> Self {
        self.name = name.into();
        self
    }
    pub fn is(&self, state: States) -> bool {
        self.states.contains(state)
    }
    pub fn accepts(&self, action: &Action) -> bool {
        self.actions.contains(action.flag()) && !self.is(States::DISABLED)
    }
}
