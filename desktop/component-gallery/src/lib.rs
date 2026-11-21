#![no_std]
#![forbid(unsafe_code)]
//! Door UI component gallery: every control of the catalog in its states,
//! on pages behind a NavigationView. The Control states page is a state
//! matrix (rest, pointer over, pressed, disabled, focused) for every
//! interactive control, with a Theme picker for light, dark and the four
//! contrast themes. It is an engineering fixture (master
//! specification sections 55 and 113), not a shell: deterministic content,
//! a deterministic clock, stored semantic trees and interaction traces
//! (`tests.rs`, `expected/`); renders are measured in the tests (and can be
//! written as BMPs for review), not stored as golden images. The Accessibility page is a
//! Narrator-style caption panel fed by the gallery's own semantic tree; the
//! `door-assist` binary shows the same captions for whatever window has
//! focus, through the door.accessibility broker.
extern crate alloc;
pub mod assist;
use alloc::{boxed::Box, format, string::String, vec, vec::Vec};
use door_accessibility::caption::Announcer;
use door_service_protocol::desktop::ContrastTheme;
use door_ui::controls::*;
use door_ui::display::Icon;
use door_ui::layout::{Align, Cell, Dim, Track};
use door_ui::{
    Insets, LayerOptions, Lu, Notification, Notify, TextRole, Theme, Ui, VisualState, Widget,
    WidgetId,
};

/// Gallery pages (NavigationView keys).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Page {
    Basic = 1,
    States = 9,
    Text = 2,
    Collections = 3,
    Menus = 4,
    Status = 5,
    Layout = 6,
    Dialogs = 7,
    Accessibility = 8,
}
impl Page {
    pub const ALL: [Page; 9] = [
        Page::Basic,
        Page::States,
        Page::Text,
        Page::Collections,
        Page::Menus,
        Page::Status,
        Page::Layout,
        Page::Dialogs,
        Page::Accessibility,
    ];
    pub fn name(self) -> &'static str {
        match self {
            Page::Basic => "Basic input",
            Page::States => "Control states",
            Page::Text => "Text and selection",
            Page::Collections => "Collections",
            Page::Menus => "Menus and toolbars",
            Page::Status => "Status and info",
            Page::Layout => "Layout",
            Page::Dialogs => "Dialogs",
            Page::Accessibility => "Accessibility",
        }
    }
    pub fn slug(self) -> &'static str {
        match self {
            Page::Basic => "basic",
            Page::States => "states",
            Page::Text => "text",
            Page::Collections => "collections",
            Page::Menus => "menus",
            Page::Status => "status",
            Page::Layout => "layout",
            Page::Dialogs => "dialogs",
            Page::Accessibility => "accessibility",
        }
    }
    fn icon(self) -> Icon {
        match self {
            Page::Basic => Icon::Home,
            Page::States => Icon::Check,
            Page::Text => Icon::Document,
            Page::Collections => Icon::Folder,
            Page::Menus => Icon::Menu,
            Page::Status => Icon::Info,
            Page::Layout => Icon::Copy,
            Page::Dialogs => Icon::Star,
            Page::Accessibility => Icon::Settings,
        }
    }
    pub fn from_key(k: u32) -> Option<Page> {
        Page::ALL.into_iter().find(|p| *p as u32 == k)
    }
}

/// Commands raised by gallery controls.
pub mod cmd {
    pub const NEW: u32 = 10;
    pub const OPEN: u32 = 11;
    pub const SAVE: u32 = 12;
    pub const CUT: u32 = 20;
    pub const COPY: u32 = 21;
    pub const PASTE: u32 = 22;
    pub const SELECT_ALL: u32 = 23;
    pub const ZOOM: u32 = 30;
    pub const STATUS_BAR: u32 = 31;
    pub const ABOUT: u32 = 40;
    pub const SHARE: u32 = 50;
    pub const DELETE: u32 = 51;
    pub const PROPERTIES: u32 = 52;
}

/// Widget ids the tests and traces address directly.
#[derive(Clone, Copy, Debug)]
pub struct Ids {
    pub nav: WidgetId,
    pub pages: [WidgetId; 9],
    pub button: WidgetId,
    pub accent: WidgetId,
    pub disabled_button: WidgetId,
    pub hyperlink: WidgetId,
    pub toggle: WidgetId,
    pub check: WidgetId,
    pub tri: WidgetId,
    pub radios: WidgetId,
    pub switch: WidgetId,
    pub slider: WidgetId,
    pub text: WidgetId,
    pub multiline: WidgetId,
    pub password: WidgetId,
    pub number: WidgetId,
    pub combo: WidgetId,
    pub list: WidgetId,
    pub multi_list: WidgetId,
    pub tree: WidgetId,
    pub menubar: WidgetId,
    pub commandbar: WidgetId,
    pub context_button: WidgetId,
    pub tabs: WidgetId,
    pub infobar: WidgetId,
    pub progress: WidgetId,
    pub tooltip_button: WidgetId,
    pub expander: WidgetId,
    pub scroller: WidgetId,
    pub dialog_button: WidgetId,
    pub dialog: Dialog,
    pub dialog_text: WidgetId,
    pub menu: WidgetId,
    pub captions: WidgetId,
    pub inspector: WidgetId,
    pub theme_picker: WidgetId,
}

pub struct Gallery {
    pub ids: Option<Ids>,
    pub page: Page,
    /// Every notification in order (stored interaction traces).
    pub log: Vec<String>,
    announcer: Announcer,
    pub captions: Vec<String>,
    next_tab: u32,
    /// Print serial markers (guest runs).
    pub markers: bool,
    /// The state matrix: row name and its cells (one per `STATES` column).
    pub matrix: Vec<(&'static str, [WidgetId; 5])>,
    /// Theme pinned by the Theme picker (`None` follows the session).
    pub theme: Option<Theme>,
}
impl Default for Gallery {
    fn default() -> Self {
        Self::new()
    }
}

fn section(ui: &mut Ui, parent: WidgetId, title: &str) -> WidgetId {
    let card = ui.add(parent, Panel::column(12).card().named(title));
    ui.add(
        card,
        TextBlock::new(title).role(TextRole::BodyStrong).heading(2),
    );
    card
}
fn row(ui: &mut Ui, parent: WidgetId) -> WidgetId {
    let r = ui.add(parent, Panel::row(12));
    ui.style_mut(r, |s| s.h_align = Align::Start);
    r
}
fn page(ui: &mut Ui, host: WidgetId, title: &str) -> (WidgetId, WidgetId) {
    let scroll = ui.add(host, ScrollViewer::vertical());
    let column = ui.add(
        scroll,
        Panel::column(16).padding(Insets::ltrb(24, 16, 24, 24)),
    );
    ui.add(
        column,
        TextBlock::new(title).role(TextRole::Title).heading(1),
    );
    (scroll, column)
}

/// Initial client size in logical units (fits a 1707x1067 logical desktop,
/// the owner's 2560x1600 panel at 150%).
pub const SIZE: (u32, u32) = (1280, 800);
/// Columns of the state matrix.
pub const STATES: [&str; 5] = ["Rest", "Pointer over", "Pressed", "Disabled", "Focused"];
/// Rows of the state matrix (`state_control` builds each).
pub const STATE_ROWS: [&str; 17] = [
    "Button",
    "Accent button",
    "Icon button",
    "Hyperlink",
    "ToggleButton",
    "ToggleButton (on)",
    "CheckBox",
    "CheckBox (checked)",
    "CheckBox (mixed)",
    "RadioButton",
    "ToggleSwitch",
    "ToggleSwitch (on)",
    "Slider",
    "TextBox",
    "PasswordBox",
    "NumberBox",
    "ComboBox",
];
/// Rows whose reference control has no separate pressed visual (WinUI text
/// inputs show the pointer-over look while pressed).
pub const NO_PRESSED_VISUAL: [&str; 3] = ["TextBox", "PasswordBox", "NumberBox"];
/// The control of a state-matrix row and its fixed width, if any.
fn state_control(row: usize) -> (Box<dyn Widget>, Option<i32>) {
    match row {
        0 => (Box::new(Button::new("Button")), None),
        1 => (Box::new(Button::new("Accent").accent()), None),
        2 => (Box::new(Button::new("").subtle().icon(Icon::Add)), None),
        3 => (Box::new(Button::new("Hyperlink").hyperlink()), None),
        4 => (Box::new(ToggleButton::new("Toggle")), None),
        5 => (Box::new(ToggleButton::new("Toggle").checked(true)), None),
        6 => (Box::new(CheckBox::new("Option")), None),
        7 => (Box::new(CheckBox::new("Option").checked(true)), None),
        8 => (Box::new(CheckBox::new("Option").indeterminate()), None),
        9 => (
            Box::new(RadioButtons::new("", &["Option"]).selected(0)),
            None,
        ),
        10 => (Box::new(ToggleSwitch::new("")), None),
        11 => (Box::new(ToggleSwitch::new("").on(true)), None),
        12 => (Box::new(Slider::new("", 0, 100, 40)), Some(120)),
        13 => (Box::new(TextBox::new("").placeholder("Text")), Some(120)),
        14 => (Box::new(TextBox::password("").text("secret")), Some(120)),
        15 => (Box::new(TextBox::number("", 0, 99, 2)), Some(110)),
        _ => (
            Box::new(ComboBox::new("", &["Item"]).selected(0)),
            Some(120),
        ),
    }
}
/// The Theme picker: the session theme, light, dark and the four contrast
/// themes.
pub const THEMES: [(&str, Option<Theme>); 7] = [
    ("Use system setting", None),
    ("Light", Some(Theme::light())),
    ("Dark", Some(Theme::dark())),
    (
        "Night sky (contrast)",
        Some(Theme::high_contrast(ContrastTheme::NightSky)),
    ),
    (
        "Aquatic (contrast)",
        Some(Theme::high_contrast(ContrastTheme::Aquatic)),
    ),
    (
        "Desert (contrast)",
        Some(Theme::high_contrast(ContrastTheme::Desert)),
    ),
    (
        "Dusk (contrast)",
        Some(Theme::high_contrast(ContrastTheme::Dusk)),
    ),
];

impl Gallery {
    /// Pin a theme from the Theme picker (`THEMES` index); 0 follows the
    /// session again.
    pub fn choose_theme(&mut self, ui: &mut Ui, index: usize) {
        let Some((name, theme)) = THEMES.get(index) else {
            return;
        };
        self.theme = *theme;
        if let Some(t) = theme {
            ui.set_theme(*t);
        }
        if self.markers {
            door_rt::diagnostic(format_args!("DOOR_GALLERY THEME {name}\n"));
        }
    }
    pub fn new() -> Self {
        Self {
            ids: None,
            page: Page::Basic,
            log: Vec::new(),
            announcer: Announcer::default(),
            captions: Vec::new(),
            next_tab: 4,
            markers: false,
            matrix: Vec::new(),
            theme: None,
        }
    }
    pub fn ids(&self) -> Ids {
        self.ids.expect("gallery built")
    }
    /// Build every page (all exist; one is visible).
    pub fn build_ui(&mut self, ui: &mut Ui) {
        ui.set_title("Doors UI Gallery");
        let root = ui.root();
        let items: Vec<NavItem> = Page::ALL[..8]
            .iter()
            .map(|p| NavItem::new(*p as u32, p.name(), p.icon()))
            .collect();
        let footer = vec![NavItem::new(
            Page::Accessibility as u32,
            Page::Accessibility.name(),
            Page::Accessibility.icon(),
        )];
        let nav = ui.add(root, NavigationView::new(items, footer));
        ui.style_mut(nav, |s| {
            s.height = Dim::Fill;
            s.grow = 1;
        });
        let host = ui.add(nav, Panel::overlay());
        ui.style_mut(host, |s| s.grow = 1);

        // Basic input.
        let (basic, col) = page(ui, host, Page::Basic.name());
        let s = section(ui, col, "Button");
        let r = row(ui, s);
        let button = ui.add(r, Button::new("Standard"));
        let accent = ui.add(r, Button::new("Accent").accent());
        let disabled_button = ui.add(r, Button::new("Disabled"));
        ui.set_enabled(disabled_button, false);
        let icon = ui.add(r, Button::new("").subtle().icon(Icon::Add));
        ui.set_name(icon, "Add item");
        let hyperlink = ui.add(r, Button::new("Hyperlink").hyperlink());
        let s = section(ui, col, "ToggleButton");
        let r = row(ui, s);
        let toggle = ui.add(r, ToggleButton::new("Bold").icon(Icon::Star));
        ui.add(r, ToggleButton::new("Checked").checked(true));
        let off = ui.add(r, ToggleButton::new("Disabled"));
        ui.set_enabled(off, false);
        let s = section(ui, col, "CheckBox");
        let r = row(ui, s);
        let check = ui.add(r, CheckBox::new("Unchecked"));
        ui.add(r, CheckBox::new("Checked").checked(true));
        let tri = ui.add(r, CheckBox::new("Indeterminate").indeterminate());
        let cd = ui.add(r, CheckBox::new("Disabled").checked(true));
        ui.set_enabled(cd, false);
        let s = section(ui, col, "RadioButtons and ToggleSwitch");
        let r = row(ui, s);
        let radios = ui.add(
            r,
            RadioButtons::new("Background", &["Picture", "Solid color", "Slideshow"]).selected(0),
        );
        let switches = ui.add(r, Panel::column(8));
        let switch = ui.add(switches, ToggleSwitch::new("Wi-Fi"));
        ui.add(switches, ToggleSwitch::new("Bluetooth").on(true));
        let sd = ui.add(switches, ToggleSwitch::new("Airplane mode"));
        ui.set_enabled(sd, false);
        let s = section(ui, col, "Slider");
        let r = row(ui, s);
        let slider = ui.add(r, Slider::new("Volume", 0, 100, 40).step(5));
        let sl = ui.add(r, Slider::new("Disabled", 0, 100, 70));
        ui.set_enabled(sl, false);

        // Control states: every interactive control in each visual state.
        let (states, col) = page(ui, host, Page::States.name());
        let s = section(ui, col, "Theme");
        let theme_picker = ui.add(
            s,
            ComboBox::new("Theme", &THEMES.map(|(name, _)| name)).selected(0),
        );
        ui.style_mut(theme_picker, |s| s.h_align = Align::Start);
        ui.add(
            s,
            TextBlock::new("Pointer over, pressed and focused cells are forced visual states (paint only); disabled cells are really disabled.")
                .tone(Tone::Secondary)
                .wrapping(),
        );
        let s = section(ui, col, "State matrix");
        let grid = ui.add(
            s,
            Panel::grid(
                vec![Track::Auto; STATES.len() + 1],
                vec![Track::Auto; STATE_ROWS.len() + 1],
                12,
            ),
        );
        ui.style_mut(grid, |s| s.h_align = Align::Start);
        let place = |ui: &mut Ui, id: WidgetId, row: usize, column: usize| {
            ui.style_mut(id, |s| {
                s.cell = Cell {
                    row: row as u16,
                    column: column as u16,
                    row_span: 1,
                    column_span: 1,
                };
                s.v_align = Align::Center;
                s.h_align = Align::Start;
            });
        };
        for (c, title) in ["Control"].iter().chain(STATES.iter()).enumerate() {
            let t = ui.add(grid, TextBlock::new(title).role(TextRole::BodyStrong));
            place(ui, t, 0, c);
        }
        self.matrix.clear();
        for (r, name) in STATE_ROWS.iter().enumerate() {
            let label = ui.add(grid, TextBlock::new(name));
            place(ui, label, r + 1, 0);
            let mut cells = [label; 5];
            for (c, state) in STATES.iter().enumerate() {
                let (control, width) = state_control(r);
                let id = ui.add_boxed(grid, control);
                if let Some(w) = width {
                    ui.style_mut(id, |s| s.width = Dim::Fixed(Lu::px(w)));
                }
                place(ui, id, r + 1, c + 1);
                ui.set_name(id, &format!("{name}, {}", state.to_lowercase()));
                ui.set_tab_stop(id, false);
                match c {
                    0 => ui.force_visual_state(id, Some(VisualState::Rest)),
                    1 => ui.force_visual_state(id, Some(VisualState::Hover)),
                    2 => ui.force_visual_state(id, Some(VisualState::Pressed)),
                    3 => ui.set_enabled(id, false),
                    _ => ui.force_visual_state(id, Some(VisualState::Focus)),
                }
                cells[c] = id;
            }
            self.matrix.push((name, cells));
        }

        // Text and selection.
        let (text_page, col) = page(ui, host, Page::Text.name());
        let s = section(ui, col, "TextBox");
        let r = row(ui, s);
        let text = ui.add(r, TextBox::new("Name").placeholder("Type your name"));
        ui.style_mut(text, |s| s.width = Dim::Fixed(Lu::px(240)));
        let filled = ui.add(r, TextBox::new("Filled").text("Caff\u{e8} latte"));
        ui.style_mut(filled, |s| s.width = Dim::Fixed(Lu::px(200)));
        let ro = ui.add(r, TextBox::new("Read only").text("Fixed value").read_only());
        ui.style_mut(ro, |s| s.width = Dim::Fixed(Lu::px(200)));
        let td = ui.add(r, TextBox::new("Disabled").text("Unavailable"));
        ui.style_mut(td, |s| s.width = Dim::Fixed(Lu::px(160)));
        ui.set_enabled(td, false);
        let multiline = ui.add(
            s,
            TextBox::multiline("Notes", 4)
                .text("First line\nSecond line with more words to wrap across the box\nThird line"),
        );
        ui.style_mut(multiline, |s| s.width = Dim::Fixed(Lu::px(480)));
        let s = section(ui, col, "PasswordBox and NumberBox");
        let r = row(ui, s);
        let password = ui.add(r, TextBox::password("Password").text("secret"));
        ui.style_mut(password, |s| s.width = Dim::Fixed(Lu::px(240)));
        let number = ui.add(r, TextBox::number("Copies", 1, 99, 2));
        ui.style_mut(number, |s| s.width = Dim::Fixed(Lu::px(160)));
        let s = section(ui, col, "ComboBox");
        let r = row(ui, s);
        let combo = ui.add(
            r,
            ComboBox::new("Theme", &["Light", "Dark", "Use system setting"]).selected(2),
        );
        ui.add(
            r,
            ComboBox::new("Font", &["Segoe UI", "Cascadia Mono", "Selawik"])
                .placeholder("Pick a font"),
        );
        let cdis = ui.add(r, ComboBox::new("Disabled", &["One"]).selected(0));
        ui.set_enabled(cdis, false);

        // Collections.
        let (collections, col) = page(ui, host, Page::Collections.name());
        let s = section(ui, col, "ListView");
        let r = row(ui, s);
        let files: Vec<ListItem> = (0..60u32)
            .map(|i| {
                ListItem::new(100 + i, &format!("Document {:02}.txt", i + 1))
                    .detail(&format!("{} KB", (i * 37) % 900 + 4))
                    .icon(Icon::Document)
            })
            .collect();
        let list = ui.add(r, ListView::new("Files", files).select(102));
        ui.style_mut(list, |s| s.width = Dim::Fixed(Lu::px(320)));
        let fruits: Vec<ListItem> = ["Apple", "Banana", "Cherry", "Date", "Elderberry"]
            .iter()
            .enumerate()
            .map(|(i, f)| ListItem::new(i as u32 + 1, f))
            .collect();
        let multi_list = ui.add(r, ListView::new("Fruits", fruits).mode(Selection::Multiple));
        ui.style_mut(multi_list, |s| s.width = Dim::Fixed(Lu::px(240)));
        ui.edit::<ListView, _>(multi_list, |l| l.selected = vec![2, 4]);
        let s = section(ui, col, "TreeView");
        let tree = ui.add(
            s,
            TreeView::new(
                "Folders",
                vec![
                    TreeItem::new(1, None, "This PC").expanded(),
                    TreeItem::new(2, Some(1), "Documents").expanded(),
                    TreeItem::new(3, Some(2), "Reports"),
                    TreeItem::new(4, Some(2), "Letters"),
                    TreeItem::new(5, Some(1), "Pictures"),
                    TreeItem::new(6, Some(5), "Holidays"),
                    TreeItem::new(7, None, "Network"),
                ],
            ),
        );
        ui.style_mut(tree, |s| s.width = Dim::Fixed(Lu::px(320)));

        // Menus and toolbars.
        let (menus, col) = page(ui, host, Page::Menus.name());
        let s = section(ui, col, "MenuBar");
        let menubar = ui.add(
            s,
            MenuBar::new(vec![
                (
                    "File",
                    vec![
                        MenuItem::new(cmd::NEW, "New")
                            .accelerator("Ctrl+N")
                            .icon(Icon::Add),
                        MenuItem::new(cmd::OPEN, "Open...")
                            .accelerator("Ctrl+O")
                            .icon(Icon::Folder),
                        MenuItem::separator(),
                        MenuItem::new(cmd::SAVE, "Save")
                            .accelerator("Ctrl+S")
                            .icon(Icon::Save),
                    ],
                ),
                (
                    "Edit",
                    vec![
                        MenuItem::new(cmd::CUT, "Cut")
                            .accelerator("Ctrl+X")
                            .icon(Icon::Cut),
                        MenuItem::new(cmd::COPY, "Copy")
                            .accelerator("Ctrl+C")
                            .icon(Icon::Copy),
                        MenuItem::new(cmd::PASTE, "Paste")
                            .accelerator("Ctrl+V")
                            .icon(Icon::Paste)
                            .disabled(),
                        MenuItem::separator(),
                        MenuItem::new(cmd::SELECT_ALL, "Select all").accelerator("Ctrl+A"),
                    ],
                ),
                (
                    "View",
                    vec![
                        MenuItem::new(cmd::ZOOM, "Zoom"),
                        MenuItem::new(cmd::STATUS_BAR, "Status bar").checked(true),
                    ],
                ),
                ("Help", vec![MenuItem::new(cmd::ABOUT, "About")]),
            ]),
        );
        let s = section(ui, col, "CommandBar");
        let commandbar = ui.add(
            s,
            CommandBar::new(
                vec![
                    Command::new(cmd::NEW, "New", Icon::Add),
                    Command::new(cmd::SAVE, "Save", Icon::Save),
                    Command::new(cmd::SHARE, "Share", Icon::Forward),
                ],
                vec![
                    MenuItem::new(cmd::DELETE, "Delete").icon(Icon::Close),
                    MenuItem::new(cmd::PROPERTIES, "Properties").icon(Icon::Info),
                ],
            ),
        );
        let s = section(ui, col, "MenuFlyout and ContextMenu");
        let context_button = ui.add(s, Button::new("Show menu (or right-click anywhere)"));
        let s = section(ui, col, "TabView");
        let tabs = ui.add(
            s,
            TabView::new(vec![
                Tab::new(1, "Home"),
                Tab::new(2, "Document 1"),
                Tab::new(3, "Document 2"),
            ]),
        );
        ui.style_mut(tabs, |s| s.height = Dim::Fixed(Lu::px(180)));
        for t in ["Home tab content", "First document", "Second document"] {
            ui.add(tabs, TextBlock::new(t));
        }

        // Status and info.
        let (status, col) = page(ui, host, Page::Status.name());
        let s = section(ui, col, "InfoBar");
        let infobar = ui.add(
            s,
            InfoBar::new(
                Severity::Informational,
                "Update available",
                "Restart to install the latest version.",
            ),
        );
        ui.add(
            s,
            InfoBar::new(Severity::Success, "Saved", "Your changes were saved."),
        );
        ui.add(
            s,
            InfoBar::new(
                Severity::Warning,
                "Low storage",
                "Free up space to keep working.",
            )
            .closable(false),
        );
        ui.add(
            s,
            InfoBar::new(
                Severity::Error,
                "Connection lost",
                "Check the network cable and try again.",
            ),
        );
        let s = section(ui, col, "ProgressBar and ProgressRing");
        let progress = ui.add(s, ProgressBar::new("Copying", 40));
        ui.add(s, ProgressBar::new("Searching", 0).indeterminate());
        ui.add(s, ProgressBar::new("Paused", 60).status(Progress::Paused));
        ui.add(s, ProgressBar::new("Failed", 75).status(Progress::Error));
        let r = row(ui, s);
        ui.add(r, ProgressRing::new("Loading").value(60));
        ui.add(r, ProgressRing::new("Working"));
        let s = section(ui, col, "ToolTip");
        let r = row(ui, s);
        let tooltip_button = ui.add(r, Button::new("Hover for a tip"));
        ui.set_tooltip(tooltip_button, "Tool tips appear after a short delay");
        ui.add(r, ToolTip::new("Static tool tip visual"));

        // Layout.
        let (layout, col) = page(ui, host, Page::Layout.name());
        let expander = ui.add(
            col,
            Expander::new("Expander").description("Show more settings"),
        );
        let inner = ui.add(expander, Panel::column(8));
        ui.add(
            inner,
            TextBlock::new("Hidden content is not published while collapsed."),
        );
        ui.add(inner, CheckBox::new("Option inside the expander"));
        let open = ui.add(col, Expander::new("Expanded expander").expanded(true));
        ui.add(open, TextBlock::new("This content is visible.").wrapping());
        let s = section(ui, col, "ScrollViewer");
        let scroller = ui.add(s, ScrollViewer::vertical());
        ui.style_mut(scroller, |s| {
            s.height = Dim::Fixed(Lu::px(160));
            s.grow = 0;
        });
        let lines = ui.add(scroller, Panel::column(4));
        for i in 1..=20 {
            ui.add(lines, TextBlock::new(&format!("Scrollable line {i}")));
        }

        // Dialogs.
        let (dialogs, col) = page(ui, host, Page::Dialogs.name());
        let s = section(ui, col, "ContentDialog");
        let dialog_button = ui.add(s, Button::new("Show dialog"));
        let dialog = ContentDialog::build(
            ui,
            "Save your work?",
            "Changes to Notes have not been saved.",
            Some("Save"),
            Some("Don't save"),
            "Cancel",
        );
        let dialog_text = ui.add(
            dialog.content,
            TextBox::new("File name").text("Untitled.txt"),
        );

        // Accessibility.
        let (access, col) = page(ui, host, Page::Accessibility.name());
        let s = section(ui, col, "Narrator captions");
        ui.add(
            s,
            TextBlock::new("What a screen reader says as focus moves (speech arrives with audio).")
                .tone(Tone::Secondary)
                .wrapping(),
        );
        let captions = ui.add(s, Panel::column(4));
        let s = section(ui, col, "Inspector");
        let inspector = ui.add(s, TextBlock::new("").wrapping());

        let menu = ui.add(
            root,
            MenuFlyout::new(vec![
                MenuItem::new(cmd::CUT, "Cut")
                    .icon(Icon::Cut)
                    .accelerator("Ctrl+X"),
                MenuItem::new(cmd::COPY, "Copy")
                    .icon(Icon::Copy)
                    .accelerator("Ctrl+C"),
                MenuItem::new(cmd::PASTE, "Paste")
                    .icon(Icon::Paste)
                    .accelerator("Ctrl+V"),
                MenuItem::separator(),
                MenuItem::new(cmd::PROPERTIES, "Properties").icon(Icon::Info),
            ]),
        );
        let pages = [
            basic,
            states,
            text_page,
            collections,
            menus,
            status,
            layout,
            dialogs,
            access,
        ];
        self.ids = Some(Ids {
            nav,
            pages,
            button,
            accent,
            disabled_button,
            hyperlink,
            toggle,
            check,
            tri,
            radios,
            switch,
            slider,
            text,
            multiline,
            password,
            number,
            combo,
            list,
            multi_list,
            tree,
            menubar,
            commandbar,
            context_button,
            tabs,
            infobar,
            progress,
            tooltip_button,
            expander,
            scroller,
            dialog_button,
            dialog,
            dialog_text,
            menu,
            captions,
            inspector,
            theme_picker,
        });
        self.show(ui, Page::Basic);
    }
    /// Show one page (others collapse: hidden content is neither painted,
    /// focusable nor published).
    pub fn show(&mut self, ui: &mut Ui, page: Page) {
        let ids = self.ids();
        for (i, p) in Page::ALL.iter().enumerate() {
            ui.set_visible(ids.pages[i], *p == page);
        }
        ui.edit::<NavigationView, _>(ids.nav, |n| n.selected = Some(page as u32));
        self.page = page;
        if self.markers {
            door_rt::diagnostic(format_args!("DOOR_GALLERY PAGE {}\n", page.slug()));
        }
    }
    /// React to one notification (models, page switches, menus, dialogs).
    pub fn handle(&mut self, ui: &mut Ui, n: Notification) {
        let ids = self.ids();
        let who = ui.type_name(n.source).unwrap_or("?");
        let entry = format!("{who} {:?}", n.kind);
        if self.markers {
            door_rt::diagnostic(format_args!("DOOR_GALLERY NOTIFY {entry}\n"));
        }
        self.log.push(entry);
        match n.kind {
            Notify::Navigated(k) => {
                if let Some(p) = Page::from_key(k) {
                    self.show(ui, p);
                }
            }
            Notify::SelectionChanged(Some(i)) if n.source == ids.theme_picker => {
                self.choose_theme(ui, i as usize)
            }
            Notify::Invoked if n.source == ids.dialog_button => {
                ContentDialog::show(ui, &ids.dialog)
            }
            Notify::Invoked if n.source == ids.context_button => {
                let r = ui.rect(ids.context_button).unwrap_or_default();
                let at = door_ui::LPoint::new(r.x, r.bottom());
                let keyboard = ui.focus_visible();
                ui.command::<MenuFlyout, _>(ids.menu, |m, cx| m.open_at(cx, at, keyboard));
            }
            Notify::ContextRequested(p) => {
                let at = ui.to_window_point(n.source, p);
                let keyboard = ui.focus_visible();
                ui.command::<MenuFlyout, _>(ids.menu, |m, cx| m.open_at(cx, at, keyboard));
            }
            Notify::TabClosed(key) => {
                if let Some(i) = ui
                    .edit::<TabView, _>(ids.tabs, |t| t.remove_tab(key))
                    .flatten()
                    && let Some(child) = ui.children(ids.tabs).get(i).copied()
                {
                    ui.remove(child);
                }
            }
            Notify::Command(TAB_ADD) => {
                let key = self.next_tab;
                self.next_tab += 1;
                let title = format!("Document {}", key - 1);
                ui.edit::<TabView, _>(ids.tabs, |t| {
                    t.tabs.push(Tab::new(key, &title));
                    t.selected = t.tabs.len() - 1;
                });
                ui.add(ids.tabs, TextBlock::new(&format!("{title} content")));
            }
            _ => (),
        }
    }
    /// Follow the semantic tree: Narrator captions and the inspector text.
    pub fn observe(&mut self, ui: &mut Ui) {
        let Some(tree) = ui.semantics().cloned() else {
            return;
        };
        let said = self.announcer.observe(&tree);
        if said.is_empty() {
            return;
        }
        for a in &said {
            if self.markers {
                // Diagnostic records hold 256 bytes.
                door_rt::diagnostic(format_args!(
                    "DOOR_GALLERY CAPTION {}\n",
                    assist::chunks(&a.text, 200).next().unwrap_or("")
                ));
            }
            self.captions.push(a.text.clone());
        }
        let ids = self.ids();
        if self.page == Page::Accessibility {
            let existing: Vec<WidgetId> = ui.children(ids.captions).to_vec();
            for c in existing {
                ui.remove(c);
            }
            for c in self.captions.iter().rev().take(6) {
                ui.add(ids.captions, TextBlock::new(c));
            }
            let i = ui.inspect();
            let text = format!(
                "{} widgets, {} display ops, {} semantic nodes, theme {}, layout passes {}, last damage {} square DIPs",
                i.widgets,
                i.display_ops,
                i.semantic_nodes,
                i.theme,
                i.stats.layout_passes,
                i.stats.damage_area_last
            );
            ui.edit::<TextBlock, _>(ids.inspector, |t| t.set_text(&text));
        }
    }
}

impl door_ui_native::App for Gallery {
    fn title(&self) -> &str {
        "Doors UI Gallery"
    }
    fn theme(&self, system: Theme) -> Theme {
        self.theme.unwrap_or(system)
    }
    fn size(&self) -> (u32, u32) {
        SIZE
    }
    fn build(&mut self, ui: &mut Ui) {
        self.markers = true;
        self.build_ui(ui);
        door_rt::diagnostic(format_args!("DOOR_GALLERY READY widgets={}\n", ui.len()));
    }
    fn notify(&mut self, ui: &mut Ui, n: Notification) {
        self.handle(ui, n);
    }
    fn trace(&mut self, ui: &mut Ui) {
        self.observe(ui);
    }
}

/// Open the dialog layer directly (tests).
pub fn open_dialog(ui: &mut Ui, d: &Dialog) {
    ui.open_layer(d.root, LayerOptions::modal());
}

#[cfg(test)]
extern crate std;
#[cfg(test)]
mod tests;
