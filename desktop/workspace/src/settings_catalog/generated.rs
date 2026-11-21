// Scaffolded from the owner's Windows 11 26H2 Settings captures (settings-parity-26h2.md);
// pages Doors lacked, then bound to Doors' providers where they exist; every
// other control is shown as currently unavailable.
use super::*;

/// Accessibility > Audio (`ms-settings:easeofaccess-audio`).
pub(super) static AUDIO: [Block; 7] = [
    Block::Intro("Make your device easier to hear or use without sound."),
    Block::Card(row!(
        "Combine left and right audio channels into one",
        "",
        "",
        Control::Toggle(Toggle::Pref {
            id: p::SOUND,
            bit: 1 << 9
        })
    )),
    Block::Card(row!(
        "Flash my screen during audio notifications",
        "",
        "",
        NA
    )),
    Block::Header("Related settings"),
    Block::Card(row!(
        "Sound",
        "Volume levels, output, input, balance level, sound devices",
        "g-e15d",
        link(Page::Sound)
    )),
    Block::Card(row!(
        "App volume and device preferences",
        "Volume levels per app, speakers",
        "g-f4c3",
        link(Page::VolumeMixer)
    )),
    Block::Card(row!("Sound themes", "", "", NA)),
];
/// Accessibility > Captions (`ms-settings:easeofaccess-closedcaptioning`).
pub(super) static CAPTIONS: [Block; 5] = [
    Block::Intro("Make your device easier to use without sound by displaying audio as text."),
    Block::Card(row!(
        "Theme preview",
        "Waves are caused by a flow of energy.",
        "",
        Control::Button("Full Screen")
    )),
    Block::Expander(Expander {
        id: 1,
        header: row!(
            "Live captions",
            "Audio and video will be captioned live on your screen",
            "",
            OFF
        ),
        hero: None,
        rows: rows![],
    }),
    Block::Card(row!("Caption style", "", "", NA)),
    Block::Card(row!("Include screen reader audio", "", "", OFF)),
];
/// Accessibility > Eye control (`ms-settings:easeofaccess-eyecontrol`).
pub(super) static EYE_CONTROL: [Block; 2] = [
    Block::Intro("Use your eyes to control your device and use text-to-speech to talk."),
    Block::Expander(Expander {
        id: 1,
        header: row!(
            "Eye control",
            "Connect an eye tracker to use eye control.",
            "",
            OFF
        ),
        hero: None,
        rows: rows![],
    }),
];
/// Accessibility > Hearing devices (`ms-settings:easeofaccess-hearingaids`).
pub(super) static HEARING_DEVICES: [Block; 4] = [
    Block::Intro("Some features might not be available for your hearing devices."),
    Block::Header("Related settings"),
    Block::Card(row!(
        "Sound",
        "Volume levels, output, input, balance level, sound devices",
        "g-e15d",
        link(Page::Sound)
    )),
    Block::Card(row!(
        "App volume and device preferences",
        "Volume levels per app, speakers",
        "g-f4c3",
        link(Page::VolumeMixer)
    )),
];
/// Accessibility > Keyboard (`ms-settings:easeofaccess-keyboard`).
pub(super) static ACCESSIBILITY_KEYBOARD: [Block; 11] = [
    Block::Intro("Make it easier to type and use keyboard shortcuts."),
    Block::Header("Sticky, Filter, and Toggle keys"),
    Block::Card(row!(
        "Sticky keys",
        "Press keyboard shortcuts that use multiple keys (such as Ctrl + S) one key at a time",
        "g-e3bb",
        OFF
    )),
    Block::Card(row!(
        "Filter keys",
        "Set the sensitivity of the keyboard so it can ignore brief or repeated keystrokes",
        "g-e3ba",
        OFF
    )),
    Block::Card(row!(
        "Play a sound when you press caps, num, or scroll lock",
        "",
        "",
        OFF
    )),
    Block::Expander(Expander {
        id: 1,
        header: row!("Notification preferences", "", "g-ea8f", NA),
        hero: None,
        rows: rows![
            row!(
                "Notify me when I turn on Sticky, Filter, or Toggle keys from the keyboard",
                "",
                "",
                NA
            ),
            row!(
                "Play a sound when I turn Sticky, Filter, or Toggle keys on or off from the keyboard",
                "",
                "",
                NA
            )
        ],
    }),
    Block::Card(row!(
        "On-screen keyboard and access keys",
        "On-screen keyboard",
        "",
        OFF
    )),
    Block::Header("Related settings"),
    Block::Card(row!("Keyboard device", "Character repeat, hotkeys", "", NA)),
    Block::Card(row!(
        "Typing",
        "Spellcheck, autocorrect, text suggestions",
        "",
        link(Page::Typing)
    )),
    Block::Card(row!(
        "Language & region",
        "Display language, preferred language, region",
        "g-f2b7",
        link(Page::LanguageRegion)
    )),
];
/// Accessibility > Magnifier (`ms-settings:easeofaccess-magnifier`).
pub(super) static MAGNIFIER: [Block; 21] = [
    Block::Expander(Expander {
        id: 1,
        header: row!(
            "Magnifier",
            "Press the Doors logo key  + the Plus sign to turn on Magnifier—and press the Doors logo key  + Esc to turn it off",
            "g-e12e",
            OFF
        ),
        hero: None,
        rows: rows![
            row!("Start Magnifier before sign-in", "", "", NA),
            row!("Start Magnifier after sign-in", "", "", NA)
        ],
    }),
    Block::Card(row!(
        "Zoom level",
        "Press Doors key + Plus (+) / Minus (-) to zoom, or Ctrl + Alt + scroll wheel",
        "",
        NA
    )),
    Block::Card(row!("View", "", "", NA)),
    Block::Header("Multiple display controls"),
    Block::Card(row!(
        "Magnify one display without changing the zoom on others",
        "",
        "",
        ON
    )),
    Block::Card(row!(
        "Lock view shortcut",
        "Press to lock the current view on a screen until unlocked",
        "g-e1f6",
        NA
    )),
    Block::Header("Appearance"),
    Block::Card(row!("Press Ctrl + Alt + I to invert colors", "", "", OFF)),
    Block::Card(row!("Smooth edges of images and text", "", "", ON)),
    Block::Header("Contrast theme"),
    Block::Card(row!(
        "Apply reduced glare color themes when using magnifier for ease of eye comfort, while photos and video keep their natural colors.",
        "",
        "",
        OFF
    )),
    Block::Card(row!("Selected theme", "", "", NA)),
    Block::Card(row!("Manage contrast theme settings", "", "", NA)),
    Block::Card(row!(
        "Show touch bars when using a touchscreen device",
        "",
        "",
        OFF
    )),
    Block::Header("Reading"),
    Block::Card(row!("Reading shortcut", "", "", NA)),
    Block::Header("Choose a voice"),
    Block::Card(row!("Speed", "", "", NA)),
    Block::Header("Related settings"),
    Block::Card(row!("Text size", "", "", link(Page::TextSize))),
    Block::Card(row!(
        "Display",
        "Monitors, brightness, night light, display profile",
        "g-e7f8",
        link(Page::Display)
    )),
];
/// Accessibility > Mouse (`ms-settings:easeofaccess-mouse`).
pub(super) static ACCESSIBILITY_MOUSE: [Block; 9] = [
    Block::Card(row!(
        "Move mouse pointer to default button when opening a supported dialog",
        "",
        "",
        OFF
    )),
    Block::Card(row!(
        "Hide mouse pointer if typing in supported apps",
        "",
        "",
        ON
    )),
    Block::Card(row!(
        "Amount of time needed between two clicks to make them a double click",
        "",
        "",
        NA
    )),
    Block::Expander(Expander {
        id: 1,
        header: row!(
            "Click lock",
            "Enables you to highlight or drag without holding down the mouse button",
            "",
            OFF
        ),
        hero: None,
        rows: rows![],
    }),
    Block::Card(row!(
        "Activate on hover",
        "Change which window receives keyboard input without clicking",
        "",
        OFF
    )),
    Block::Card(row!(
        "Mouse keys",
        "Use your numeric keypad to move your mouse pointer",
        "g-e962",
        OFF
    )),
    Block::Header("Related settings"),
    Block::Card(row!(
        "Mouse pointer",
        "Mouse pointer color, size",
        "g-e1e3",
        NA
    )),
    Block::Card(row!(
        "Mouse",
        "Mouse pointer speed, primary button, scrolling",
        "g-e962",
        link(Page::Mouse)
    )),
];
/// Accessibility > Mouse pointer and touch (`ms-settings:easeofaccess-mousepointer`).
pub(super) static MOUSE_POINTER_AND_TOUCH: [Block; 30] = [
    Block::Header("Mouse pointer"),
    Block::Expander(Expander {
        id: 1,
        header: row!("Mouse pointer style", "", "", Control::None),
        hero: None,
        rows: rows![],
    }),
    Block::Card(row!("Size", "", "", NA)),
    Block::Header("Customize pointer image"),
    Block::Card(row!("Normal select", "", "", Control::Button("Browse"))),
    Block::Card(row!("Text select", "", "", Control::Button("Browse"))),
    Block::Card(row!("Link select", "", "", Control::Button("Browse"))),
    Block::Card(row!("Busy", "", "", Control::Button("Browse"))),
    Block::Card(row!("Vertical resize", "", "", Control::Button("Browse"))),
    Block::Card(row!("Horizontal resize", "", "", Control::Button("Browse"))),
    Block::Card(row!("Diagonal resize 1", "", "", Control::Button("Browse"))),
    Block::Card(row!("Diagonal resize 2", "", "", Control::Button("Browse"))),
    Block::Card(row!("Move", "", "", Control::Button("Browse"))),
    Block::Card(row!("Help select", "", "", Control::Button("Browse"))),
    Block::Card(row!("Unavailable", "", "", Control::Button("Browse"))),
    Block::Card(row!(
        "Working in background",
        "",
        "",
        Control::Button("Browse")
    )),
    Block::Card(row!("Alternate select", "", "", Control::Button("Browse"))),
    Block::Card(row!("Precision select", "", "", Control::Button("Browse"))),
    Block::Card(row!("Pen", "", "", Control::Button("Browse"))),
    Block::Card(row!("Location select", "", "", Control::Button("Browse"))),
    Block::Card(row!("Person select", "", "", Control::Button("Browse"))),
    Block::Card(row!(
        "Show a circle around the mouse pointer when I press the Ctrl key",
        "",
        "",
        OFF
    )),
    Block::Card(row!(
        "Mouse pointer trails",
        "Make mouse pointer movement easier to see by showing previous positions for a short time",
        "",
        OFF
    )),
    Block::Card(row!("Show shadow under the mouse pointer", "", "", OFF)),
    Block::Card(row!(
        "Touch indicator",
        "Show a circle on the screen where I touch it",
        "g-e815",
        ON
    )),
    Block::Header("Related settings"),
    Block::Card(row!(
        "Mouse",
        "Mouse pointer speed, primary button, scrolling",
        "g-e962",
        link(Page::Mouse)
    )),
    Block::Card(row!(
        "Text cursor",
        "Text cursor indicator, thickness",
        "",
        link(Page::TextCursor)
    )),
    Block::Card(row!(
        "Touchpad",
        "Gestures, sensitivity",
        "",
        link(Page::Touchpad)
    )),
    Block::Card(row!(
        "Themes",
        "Background image, color, theme packs",
        "g-e771",
        link(Page::Themes)
    )),
];
/// Accessibility > Narrator (`ms-settings:easeofaccess-narrator-extensions`).
pub(super) static NARRATOR: [Block; 39] = [
    Block::Intro("Narrator is a screen reader that describes what's on your screen."),
    Block::Header("Use Narrator"),
    Block::Expander(Expander {
        id: 1,
        header: row!("Narrator", "", "g-ed4d", OFF),
        hero: None,
        rows: rows![
            row!("Start Narrator before sign-in", "", "", NA),
            row!("Start Narrator after sign-in", "", "", NA)
        ],
    }),
    Block::Card(row!(
        "Keyboard shortcut for Narrator",
        "Press the Doors logo key  + Ctrl + Enter to turn Narrator on and off",
        "",
        ON
    )),
    Block::Header("Narrator Home"),
    Block::Card(row!("Open Narrator Home", "", "", NA)),
    Block::Card(row!("What's new in Narrator", "", "g-e789", NA)),
    Block::Card(row!("Complete guide to Narrator", "", "g-e9ce", NA)),
    Block::Header("Narrator's voice"),
    Block::Header("Choose a voice"),
    Block::Card(row!(
        "Speed",
        "Press Narrator + the Plus sign or Narrator + the Minus sign to change voice speed",
        "",
        NA
    )),
    Block::Card(row!("Pitch", "", "", NA)),
    Block::Card(row!(
        "Volume",
        "Press Narrator + Ctrl + the Plus sign or Narrator + Ctrl + the Minus sign to change voice volume",
        "g-e15d",
        NA
    )),
    Block::Card(row!(
        "Lower the volume of other apps when Narrator is speaking",
        "",
        "",
        ON
    )),
    Block::Card(row!("Narrator audio output device", "", "", NA)),
    Block::Header("Verbosity"),
    Block::Card(row!(
        "Verbosity level",
        "Press Narrator + V to change verbosity",
        "",
        NA
    )),
    Block::Card(row!(
        "Context level for buttons and controls",
        "Examples: turning scan mode on and off, landing on a link",
        "",
        NA
    )),
    Block::Card(row!("Have Narrator announce when I type", "", "", NA)),
    Block::Header("Mouse and keyboard"),
    Block::Card(row!("Narrator key", "", "", NA)),
    Block::Card(row!(
        "On touch keyboards, activate keys when I lift my finger",
        "",
        "",
        OFF
    )),
    Block::Card(row!(
        "Read and interact with the screen using the mouse",
        "",
        "g-e962",
        OFF
    )),
    Block::Card(row!("Keyboard layout", "", "g-f73f", NA)),
    Block::Card(row!("Create a custom keyboard layout", "", "", NA)),
    Block::Header("Narrator cursor"),
    Block::Card(row!(
        "The Narrator cursor is where Narrator is focused on your screen",
        "",
        "",
        ON
    )),
    Block::Card(row!(
        "Move my text cursor with the Narrator cursor as Narrator reads text",
        "",
        "",
        OFF
    )),
    Block::Card(row!(
        "Sync the Narrator cursor and system focus",
        "",
        "",
        ON
    )),
    Block::Card(row!(
        "Navigation mode",
        "Normal mode is recommended",
        "g-e742",
        NA
    )),
    Block::Header("Math"),
    Block::Card(row!(
        "Math reading",
        "Configure how Narrator reads mathematical content",
        "g-ee56",
        link(Page::MathReading)
    )),
    Block::Header("Braille"),
    Block::Card(row!(
        "Configure braille displays",
        "Enable braille, displays and drivers, braille input and output",
        "",
        link(Page::ConfigureBrailleDisplays)
    )),
    Block::Card(row!(
        "Enable Narrator extensions",
        "Extensions customize the Narrator experience in some apps",
        "",
        ON
    )),
    Block::Header("Manage data and services"),
    Block::Card(row!(
        "Anyone who signs into this PC can use and edit these settings (recommended for serial braille display users)",
        "",
        "",
        Control::Button("Sync settings")
    )),
    Block::Card(row!(
        "Press Narrator + Ctrl + D for an image description or page title—press Narrator + S twice quickly for a list of popular links",
        "",
        "",
        ON
    )),
    Block::Card(row!(
        "Restart Narrator after turning this on—press Narrator + Alt + F to give feedback",
        "",
        "",
        OFF
    )),
];
/// Accessibility > Narrator > Configure braille displays (`ms-settings:easeofaccess-narrator-usebrailledisplay`).
pub(super) static CONFIGURE_BRAILLE_DISPLAYS: [Block; 13] = [
    Block::Intro("The following settings allow you to add and configure braille displays."),
    Block::Expander(Expander {
        id: 1,
        header: row!(
            "Enable braille",
            "Use braille displays with USB or Bluetooth (HID)",
            "",
            OFF
        ),
        hero: None,
        rows: rows![],
    }),
    Block::Card(row!(
        "Braille displays and drivers",
        "Braille displays",
        "",
        NA
    )),
    Block::Header("Braille display input and output"),
    Block::Card(row!("Input language", "", "g-f2b7", NA)),
    Block::Card(row!("Input table type", "", "", NA)),
    Block::Card(row!("Output language", "", "g-f2b7", NA)),
    Block::Card(row!("Output table type", "", "", NA)),
    Block::Header("Notifications"),
    Block::Card(row!("Notifications timeout after", "", "", NA)),
    Block::Header("Braille cursor"),
    Block::Card(row!("Display the text cursor", "", "g-eccc", NA)),
    Block::Card(row!("Blink cursor on braille display", "", "", ON)),
];
/// Accessibility > Narrator > Math reading (`ms-settings:easeofaccess-narrator-mathreading`).
pub(super) static MATH_READING: [Block; 4] = [
    Block::Intro(
        "The following settings allow you to configure how mathematical content is spoken by Narrator.",
    ),
    Block::Card(row!("Enable math reading", "", "", ON)),
    Block::Card(row!("Math speech", "Language", "", NA)),
    Block::Card(row!("Math navigation", "Navigation mode", "", ON)),
];
/// Accessibility > Screen tint (`ms-settings:easeofaccess-screentint`).
pub(super) static SCREEN_TINT: [Block; 3] = [
    Block::Card(row!(
        "Show a color overlay on your display to reduce eye strain and improve viewing comfort",
        "",
        "",
        OFF
    )),
    Block::Card(row!("Custom tint", "", "", Control::Button("View colors"))),
    Block::Card(row!("Strength", "", "", NA)),
];
/// Accessibility > Speech (`ms-settings:easeofaccess-speechrecognition`).
pub(super) static ACCESSIBILITY_SPEECH: [Block; 4] = [
    Block::Intro("Use your voice to type and control your device."),
    Block::Expander(Expander {
        id: 1,
        header: row!(
            "Voice access",
            "Modern voice command feature to help you interact with your PC and dictate text",
            "",
            OFF
        ),
        hero: None,
        rows: rows![
            row!(
                "Start voice access before you sign in to your PC",
                "",
                "",
                NA
            ),
            row!(
                "Start voice access after you sign in to your PC",
                "",
                "",
                NA
            )
        ],
    }),
    Block::Header("Related settings"),
    Block::Card(row!(
        "Sound",
        "Volume levels, output, input, sound devices",
        "g-e15d",
        link(Page::Sound)
    )),
];
/// Accessibility > Text cursor (`ms-settings:easeofaccess-cursor`).
pub(super) static TEXT_CURSOR: [Block; 5] = [
    Block::Expander(Expander {
        id: 1,
        header: row!("Text cursor indicator", "Size", "", OFF),
        hero: None,
        rows: rows![],
    }),
    Block::Card(row!(
        "Text cursor thickness",
        "Text cursor thickness preview",
        "",
        NA
    )),
    Block::Card(row!(
        "Cursor blink rate",
        "Cursor blink rate preview",
        "",
        NA
    )),
    Block::Header("Related settings"),
    Block::Card(row!(
        "Mouse pointer and touch",
        "Mouse pointer color and size, touch indicator",
        "g-e1e3",
        link(Page::MousePointerAndTouch)
    )),
];
/// Accounts > Access work or school (`ms-settings:workplace`).
pub(super) static ACCESS_WORK_OR_SCHOOL: [Block; 5] = [
    Block::Card(row!(
        "Add a work or school account",
        "",
        "",
        Control::Button("Connect")
    )),
    Block::Header("Related settings"),
    Block::Card(row!(
        "Create a test-taking account",
        "Choose an account for the test taker and enter the address",
        "",
        NA
    )),
    Block::Card(row!("Add or remove a provisioning package", "", "", NA)),
    Block::Card(row!("Enroll only in device management", "", "", NA)),
];
/// Accounts > Other Users > Kiosk (`ms-settings:assignedaccess`).
pub(super) static KIOSK: [Block; 1] = [Block::Intro(
    "To set up this device as a kiosk, navigate to “Other Users” to create or select a local account and then choose the only app that it can use (assigned access).",
)];
/// Accounts > Passkeys (`ms-settings:savedpasskeys`).
pub(super) static ACCOUNTS_PASSKEYS: [Block; 2] = [
    Block::Card(row!(
        "Advanced options",
        "Manage where your passkeys are saved",
        "",
        link(Page::PasskeysAdvancedOptions)
    )),
    Block::Header("Saved passkeys"),
];
/// Accounts > Passkeys > Advanced options (`ms-settings:passkeys-advancedoptions`).
pub(super) static PASSKEYS_ADVANCED_OPTIONS: [Block; 3] = [
    Block::Header("Passkey managers"),
    Block::Card(row!(
        "Macrohard Password Manager",
        "Macrohard Corporation",
        "",
        OFF
    )),
    Block::Card(row!("Save passkeys to this Doors device", "", "", ON)),
];
/// Accounts > Your accounts (`ms-settings:emailandaccounts`).
pub(super) static YOUR_ACCOUNTS: [Block; 0] = [];
/// Apps > Advanced app settings (`ms-settings:advanced-apps`).
pub(super) static ADVANCED_APP_SETTINGS: [Block; 8] = [
    Block::Card(row!("Choose where to get apps", "", "", NA)),
    Block::Header("Share across devices"),
    Block::Card(row!("Share across devices", "", "", Control::None)),
    Block::Card(row!(
        "Automatically archive your infrequently used apps to save storage space and internet bandwidth. Your files and data will still be saved, and the app's full version will be restored on your next use if it's still available.",
        "",
        "",
        ON
    )),
    Block::Card(row!("App execution aliases", "", "", NA)),
    Block::Header("Related settings"),
    Block::Card(row!(
        "Uninstall updates",
        "",
        "",
        link(Page::UninstallUpdates)
    )),
    Block::Card(row!("How to install a program", "", "", NA)),
];
/// Apps > Apps for websites (`ms-settings:appsforwebsites`).
pub(super) static APPS_FOR_WEBSITES: [Block; 1] =
    [Block::Header("Open links in an app instead of a browser")];
/// Apps > Offline maps (`ms-settings:maps-downloadmaps`).
pub(super) static OFFLINE_MAPS: [Block; 0] = [];
/// Apps > Resume (`ms-settings:system-resume`).
pub(super) static RESUME: [Block; 3] = [
    Block::Card(row!(
        "Resume",
        "Start something on one device and continue on this PC",
        "g-ea6c",
        OFF
    )),
    Block::Header("Related settings"),
    Block::Card(row!(
        "Taskbar",
        "Taskbar behaviors, system pins",
        "g-e147",
        link(Page::Taskbar)
    )),
];
/// Apps > Startup (`ms-settings:startupapps`).
pub(super) static STARTUP: [Block; 2] = [
    Block::Intro(
        "Apps can be configured to start when you log in. In most cases, apps will start minimized or may only start a background task.",
    ),
    Block::Note("No apps are set to start when you sign in."),
];
/// Apps > Video playback (`ms-settings:videoplayback`).
pub(super) static VIDEO_PLAYBACK: [Block; 7] = [
    Block::Card(row!(
        "Process video automatically to enhance it (depends on your device hardware)",
        "",
        "",
        OFF
    )),
    Block::Card(row!(
        "Save network bandwidth by playing video at a lower resolution",
        "",
        "",
        OFF
    )),
    Block::Expander(Expander {
        id: 1,
        header: row!(
            "Battery options",
            "When watching movies and videos on battery power",
            "g-ebb2",
            NA
        ),
        hero: None,
        rows: rows![row!(
            "Play video at a lower resolution when on battery",
            "",
            "",
            NA
        )],
    }),
    Block::Header("Related settings"),
    Block::Card(row!(
        "HDR",
        "Display capabilities, use HDR, Auto HDR",
        "",
        link(Page::HDR)
    )),
    Block::Card(row!(
        "Display",
        "Display information",
        "g-e7f8",
        link(Page::Display)
    )),
    Block::Card(row!(
        "Power & battery",
        "Screen and sleep, battery usage, power mode, energy saver",
        "g-e7e8",
        NA
    )),
];
/// Bluetooth & devices > AutoPlay (`ms-settings:autoplay`).
pub(super) static AUTO_PLAY: [Block; 1] = [Block::Card(row!(
    "Choose AutoPlay defaults",
    "Removable drive",
    "",
    NA
))];
/// Bluetooth & devices > Cameras (`ms-settings:camera`).
pub(super) static CAMERAS: [Block; 3] = [
    Block::Card(row!(
        "Add a network camera",
        "",
        "g-ecc8",
        Control::Button("Search for cameras")
    )),
    Block::Header("Related settings"),
    Block::Card(row!("Camera privacy settings", "", "", NA)),
];
/// Bluetooth & devices > Keyboard (`ms-settings:devices-keyboard`).
pub(super) static DEVICES_KEYBOARD: [Block; 7] = [
    Block::Card(row!(
        "Keyboard character repeat",
        "Repeat delay and repeat rate preview",
        "",
        NA
    )),
    Block::Card(row!("Shortcuts and hotkeys", "", "", ON)),
    Block::Header("Related settings"),
    Block::Card(row!(
        "Keyboard accessibility",
        "Sticky, Filter, and Toggle keys, on-screen keyboard",
        "",
        NA
    )),
    Block::Card(row!(
        "Text input personalization",
        "Touch keyboard, voice typing, emoji and more, input method editor",
        "",
        NA
    )),
    Block::Card(row!(
        "Dynamic Lighting",
        "Connected devices, effects, app settings",
        "",
        link(Page::DynamicLighting)
    )),
    Block::Card(row!("Spelling, typing, & keyboard settings", "", "", NA)),
];
/// Bluetooth & devices > Pen & Windows Ink (`ms-settings:pen-button`).
pub(super) static PEN_AND_WINDOWS_INK: [Block; 1] = [Block::Expander(Expander {
    id: 1,
    header: row!(
        "Handwriting panel settings",
        "Choose how you write and how your text appears when you use the handwriting panel",
        "",
        NA
    ),
    hero: None,
    rows: rows![row!("Write with your fingertip", "", "", NA)],
})];
/// Bluetooth & devices > Printers & scanners (`ms-settings:printers-autodiscovery`).
pub(super) static PRINTERS_AND_SCANNERS: [Block; 7] = [
    Block::Card(row!(
        "Add a printer or scanner",
        "",
        "",
        Control::Button("Refresh")
    )),
    Block::Header("Printer preferences"),
    Block::Card(row!(
        "Doors protected print mode",
        "This feature helps ensure the latest security standard for printer software. Turning this on may limit the availability of some printers using an older default security standard.",
        "",
        Control::Button("Set up")
    )),
    Block::Card(row!("Let Doors manage my default printer", "", "", ON)),
    Block::Card(row!(
        "Download drivers and device software over metered connections",
        "Data charges may apply",
        "",
        OFF
    )),
    Block::Header("Related settings"),
    Block::Card(row!("Print server properties", "", "", NA)),
];
/// Bluetooth & devices > Touchpad (`ms-settings:devices-touchpad`).
pub(super) static TOUCHPAD: [Block; 6] = [
    Block::Intro("Your device has a precision touchpad."),
    Block::Expander(Expander {
        id: 1,
        header: row!(
            "Touchpad",
            "Reset touchpad settings and gestures to defaults",
            "",
            ON
        ),
        hero: None,
        rows: rows![row!(
            "Leave touchpad on when a mouse is connected",
            "",
            "",
            NA
        )],
    }),
    Block::Expander(Expander {
        id: 2,
        header: row!("Gestures & interaction", "Cursor speed", "", NA),
        hero: None,
        rows: rows![
            row!("Tap with a single finger to single-click", "", "", NA),
            row!("Tap with two fingers to right-click", "", "", NA),
            row!("Tap twice and drag to multi-select", "", "", NA),
            row!(
                "Press the lower right corner of the touchpad to right-click",
                "",
                "",
                NA
            ),
            row!("Drag two fingers to scroll", "", "", NA),
            row!("Edge scrolling", "", "", NA),
            row!("Accelerated scrolling", "", "", NA),
            row!("Pinch to zoom", "", "", NA)
        ],
    }),
    Block::Header("Related settings"),
    Block::Card(row!(
        "More touchpad settings",
        "Pointer icons and visibility",
        "",
        NA
    )),
    Block::Card(row!("Advanced gestures", "", "", NA)),
];
/// Bluetooth & devices > USB (`ms-settings:usb`).
pub(super) static USB: [Block; 2] = [
    Block::Card(row!(
        "Show a notification if there are issues connecting a USB device",
        "",
        "",
        ON
    )),
    Block::Card(row!(
        "Stop USB devices when my screen is off to help save battery",
        "",
        "",
        ON
    )),
];
/// Gaming > Game Mode (`ms-settings:gaming-gamemode`).
pub(super) static GAME_MODE: [Block; 2] = [
    Block::Header("Related settings"),
    Block::Card(row!("Graphics", "", "g-f211", link(Page::Graphics))),
];
/// Network & internet > Advanced network settings > Advanced sharing settings (`ms-settings:network-advancedsharing`).
pub(super) static ADVANCED_SHARING_SETTINGS: [Block; 3] = [
    Block::Expander(Expander {
        id: 1,
        header: row!("Private networks", "Network discovery", "", ON),
        hero: None,
        rows: rows![
            row!("Network discovery", "", "", ON),
            row!("Set up network connected devices automatically", "", "", NA),
            row!("File and printer sharing", "", "", OFF)
        ],
    }),
    Block::Card(row!("Public networks", "Current profile", "", OFF)),
    Block::Card(row!("All networks", "Public folder sharing", "", OFF)),
];
/// Network & internet > Airplane mode (`ms-settings:network-airplanemode`).
pub(super) static AIRPLANE_MODE: [Block; 3] = [
    Block::Card(row!("Airplane mode", "", "", OFF)),
    Block::Card(row!("Wi-Fi", "", "", ON)),
    Block::Card(row!("Bluetooth", "", "", ON)),
];
/// Network & internet > Dial-up (`ms-settings:network-dialup`).
pub(super) static DIAL_UP: [Block; 0] = [];
/// Network & internet > Ethernet (`ms-settings:network-ethernet`).
pub(super) static ETHERNET: [Block; 1] = [Block::Expander(Expander {
    id: 1,
    header: row!(
        "Ethernet",
        "Authentication, IP and DNS settings, metered network",
        "g-e839",
        Control::Value(Fact::Net(NetFact::State))
    ),
    hero: None,
    rows: rows![
        row!("Metered connection", "Currently unavailable", "", OFF),
        row!(
            "IP assignment",
            "Automatic (DHCP) or a manual address",
            "",
            Control::Command("Edit", Command::EditIp)
        ),
        row!(
            "DNS server assignment",
            "Automatic (DHCP) or manual DNS servers",
            "",
            Control::Command("Edit", Command::EditDns)
        ),
        row!(
            "IPv6 address",
            "",
            "",
            Control::Value(Fact::Net(NetFact::Ipv6Address))
        ),
        row!(
            "Link-local IPv6 address",
            "",
            "",
            Control::Value(Fact::Net(NetFact::LinkLocal6))
        ),
        row!(
            "IPv6 DNS servers",
            "",
            "",
            Control::Value(Fact::Net(NetFact::Dns6))
        ),
        row!(
            "IPv4 address",
            "",
            "",
            Control::Value(Fact::Net(NetFact::Address))
        ),
        row!(
            "Default gateway",
            "",
            "",
            Control::Value(Fact::Net(NetFact::Gateway))
        ),
        row!(
            "IPv4 DNS servers",
            "",
            "",
            Control::Value(Fact::Net(NetFact::Dns))
        ),
        row!(
            "Description",
            "",
            "",
            Control::Value(Fact::Net(NetFact::Adapter))
        ),
        row!(
            "Physical address (MAC)",
            "",
            "",
            Control::Value(Fact::Net(NetFact::Mac))
        ),
        row!(
            "Data usage",
            "",
            "",
            Control::Value(Fact::Net(NetFact::Traffic))
        )
    ],
})];
/// Network & internet > Mobile hotspot (`ms-settings:network-mobilehotspot`).
pub(super) static MOBILE_HOTSPOT: [Block; 4] = [
    Block::Card(row!("Mobile hotspot", "", "", OFF)),
    Block::Card(row!("Share my internet connection from", "", "", NA)),
    Block::Card(row!("Share over", "", "", NA)),
    Block::Expander(Expander {
        id: 1,
        header: row!(
            "Properties",
            "Network properties",
            "",
            Control::Button("Edit network info")
        ),
        hero: None,
        rows: rows![],
    }),
];
/// Network & internet > Wi-Fi (`ms-settings:network-wifi`).
pub(super) static WI_FI: [Block; 4] = [
    Block::Header("Wi-Fi"),
    Block::Expander(Expander {
        id: 1,
        header: row!("Show available networks", "", "g-ec05", Control::None),
        hero: None,
        rows: rows![],
    }),
    Block::Card(row!(
        "Manage known networks",
        "Add, remove, and edit networks",
        "g-e292",
        link(Page::ManageKnownNetworks)
    )),
    Block::Card(row!(
        "Help protect your privacy by making it harder for people to track your device location.",
        "",
        "",
        OFF
    )),
];
/// Network & internet > Wi-Fi > Manage known networks (`ms-settings:network-wifisettings`).
pub(super) static MANAGE_KNOWN_NETWORKS: [Block; 2] = [
    Block::Card(row!("Known networks", "", "", NA)),
    Block::Header("Add a new network"),
];
/// Personalization > Dynamic Lighting (`ms-settings:personalization-lighting`).
pub(super) static DYNAMIC_LIGHTING: [Block; 7] = [
    Block::Header("Manage all Dynamic Lighting devices"),
    Block::Card(row!("Use Dynamic Lighting on my devices", "", "", ON)),
    Block::Card(row!(
        "Compatible apps in the foreground always control lighting",
        "",
        "",
        ON
    )),
    Block::Header("Background light control"),
    Block::Card(row!(
        "Dynamic Lighting Background Controller",
        "",
        "",
        Control::None
    )),
    Block::Card(row!("Change the brightness of your lights", "", "", NA)),
    Block::Card(row!(
        "Effects",
        "Choose color themes and effects for your lighting",
        "g-e771",
        ON
    )),
];
/// Personalization > Start > Folders (`ms-settings:personalization-start-places`).
pub(super) static FOLDERS: [Block; 10] = [
    Block::Intro("Choose which folders appear on Start next to the Power button."),
    Block::Card(row!("Settings", "", "settings", folder_toggle(0))),
    Block::Card(row!("File Explorer", "", "folder", folder_toggle(1))),
    Block::Card(row!("Documents", "", "g-e130", folder_toggle(2))),
    Block::Card(row!("Downloads", "", "g-e118", folder_toggle(3))),
    Block::Card(row!("Music", "", "g-e8d6", folder_toggle(4))),
    Block::Card(row!("Pictures", "", "g-e91b", folder_toggle(5))),
    Block::Card(row!("Videos", "", "g-e714", folder_toggle(6))),
    Block::Card(row!("Network", "", "network", folder_toggle(7))),
    Block::Card(row!("Personal folder", "", "people", folder_toggle(8))),
];
/// Personalization > Text input (`ms-settings:personalization-textinput`).
/// Touch keyboard / emoji panel themes (26H2 names); Doors has neither
/// surface yet, so the list shows System default, unavailable.
const TEXT_INPUT_THEMES: [&str; 16] = [
    "System default",
    "Light",
    "Dark",
    "Color Pop",
    "Black-white",
    "Poppy Red",
    "Ice Blue",
    "Platinum",
    "Tangerine Tides",
    "Lilac River",
    "Silky Dawn",
    "Indigo Breeze",
    "Pink-blue",
    "Green-purple",
    "Pink-orange",
    "Blue-green",
];
const SIZES: [&str; 3] = ["Small", "Medium", "Large"];
pub(super) static TEXT_INPUT: [Block; 15] = [
    Block::Intro(
        "Choose a theme for touch keyboard, voice typing, emoji and more, and input method editors.",
    ),
    Block::Card(row!(
        "Theme",
        "",
        "personalize",
        Control::Combo(Choice::Unavailable(&TEXT_INPUT_THEMES, 0))
    )),
    Block::Header("Touch keyboard"),
    Block::Card(row!("Keyboard size", "", "keyboard", NA)),
    Block::Card(row!("Show key background", "", "", ON)),
    Block::Card(row!(
        "Key text size",
        "",
        "",
        Control::Combo(Choice::Unavailable(&SIZES, 0))
    )),
    Block::Card(row!(
        "View your changes",
        "",
        "",
        Control::Button("Open keyboard")
    )),
    Block::Card(row!(
        "Show touch keyboard icon in taskbar",
        "",
        "",
        link(Page::Taskbar)
    )),
    Block::Header("Candidate window (Simplified Chinese)"),
    Block::Card(row!(
        "Height",
        "",
        "",
        Control::Combo(Choice::Unavailable(&SIZES, 0))
    )),
    Block::Card(row!(
        "Text size",
        "",
        "",
        Control::Combo(Choice::Unavailable(&SIZES, 0))
    )),
    Block::Card(row!("More settings", "", "", link(Page::MicrosoftPinyin))),
    Block::Header("Related settings"),
    Block::Card(row!(
        "Keyboard device",
        "Character repeat, hotkeys",
        "keyboard",
        link(Page::DevicesKeyboard)
    )),
    Block::Card(row!(
        "Typing settings",
        "Spelling, touch keyboard, hardware keyboard",
        "keyboard",
        link(Page::Typing)
    )),
];
/// Privacy & security > Account info (`ms-settings:privacy-accountinfo`).
pub(super) static ACCOUNT_INFO: [Block; 2] = [
    Block::Card(row!(
        "Account info access",
        "Anyone using this device can choose if their apps have access to their account info when this is on",
        "",
        OFF
    )),
    Block::Card(row!(
        "Let apps access your account info",
        "Choose which apps can access your account info",
        "",
        OFF
    )),
];
/// Privacy & security > App diagnostics (`ms-settings:privacy-appdiagnostics`).
pub(super) static APP_DIAGNOSTICS: [Block; 2] = [
    Block::Card(row!(
        "App diagnostic access",
        "Anyone using this device can choose if their apps have access to diagnostic info about other apps when this is on",
        "",
        ON
    )),
    Block::Card(row!(
        "Let apps access diagnostic info about your other apps",
        "Choose which apps can access diagnostic info about your other apps",
        "",
        ON
    )),
];
/// Privacy & security > Automatic file downloads (`ms-settings:privacy-automaticfiledownloads`).
pub(super) static AUTOMATIC_FILE_DOWNLOADS: [Block; 1] = [Block::Card(row!(
    "Allow previously blocked apps to request automatic file downloads",
    "",
    "",
    Control::Button("Allow")
))];
/// Privacy & security > Calendar (`ms-settings:privacy-calendar`).
pub(super) static CALENDAR: [Block; 3] = [
    Block::Card(row!(
        "Calendar access",
        "Anyone using this device can choose if their apps have access to their calendar when this is on",
        "",
        ON
    )),
    Block::Header("Let apps access your calendar"),
    Block::Card(row!(
        "Choose which apps can access your calendar",
        "",
        "",
        ON
    )),
];
/// Privacy & security > Call history (`ms-settings:privacy-callhistory`).
pub(super) static CALL_HISTORY: [Block; 3] = [
    Block::Card(row!(
        "Call history access",
        "Anyone using this device can choose if their apps have access to their call history when this is on",
        "g-e81c",
        ON
    )),
    Block::Header("Let apps access your call history"),
    Block::Card(row!(
        "Choose which apps can access your call history",
        "",
        "",
        ON
    )),
];
/// Privacy & security > Calls (`ms-settings:privacy-phonecalls`).
pub(super) static CALLS: [Block; 3] = [
    Block::Card(row!(
        "Phone call access",
        "Anyone using this device can choose if their apps can make phone calls when this is on",
        "",
        ON
    )),
    Block::Header("Let apps make phone calls"),
    Block::Card(row!("Choose which apps can make phone calls", "", "", ON)),
];
/// Privacy & security > Camera (`ms-settings:privacy-webcam-spotlightactivity`).
pub(super) static CAMERA: [Block; 7] = [
    Block::Card(row!(
        "Camera access",
        "Anyone using this device can choose if their apps have camera access when this is on",
        "",
        ON
    )),
    Block::Header("Let apps access your camera"),
    Block::Card(row!("Choose which apps can access your camera", "", "", ON)),
    Block::Card(row!(
        "Desktop apps that have previously accessed your camera are listed here",
        "",
        "",
        ON
    )),
    Block::Header("Related settings"),
    Block::Card(row!("Camera device settings", "", "", NA)),
    Block::Card(row!("Sign-in options and facial recognition", "", "", NA)),
];
/// Privacy & security > Contacts (`ms-settings:privacy-contacts`).
pub(super) static CONTACTS: [Block; 3] = [
    Block::Card(row!(
        "Contacts access",
        "Anyone using this device can choose if their apps have access to their contacts when this is on",
        "g-e125",
        OFF
    )),
    Block::Header("Let apps access your contacts"),
    Block::Card(row!(
        "Choose which apps can access your contacts",
        "",
        "",
        OFF
    )),
];
/// Privacy & security > Custom dictionary (`ms-settings:privacy-speechtyping`).
pub(super) static CUSTOM_DICTIONARY: [Block; 3] = [
    Block::Intro(
        "Get better suggestions by creating a custom dictionary using your typing and handwriting info. If you turn off this setting, your dictionary will be cleared and your suggestions won’t be personalized.",
    ),
    Block::Card(row!(
        "Use my typing history and handwriting patterns to create a custom dictionary",
        "",
        "",
        OFF
    )),
    Block::Card(row!("My custom dictionary", "", "", NA)),
];
/// Privacy & security > Documents (`ms-settings:privacy-documents`).
pub(super) static DOCUMENTS: [Block; 3] = [
    Block::Card(row!(
        "Documents library access",
        "Anyone using this device can choose if their apps have access to their documents library when this is on",
        "",
        ON
    )),
    Block::Header("Let apps access your documents library"),
    Block::Card(row!(
        "Choose which apps can access your documents library",
        "",
        "",
        ON
    )),
];
/// Privacy & security > Downloads (`ms-settings:privacy-downloadsfolder`).
pub(super) static DOWNLOADS: [Block; 2] = [
    Block::Card(row!(
        "Downloads folder access",
        "Anyone using this device can choose if their apps have access to their downloads folder when this is on",
        "g-ea1a",
        ON
    )),
    Block::Card(row!(
        "Let apps access your downloads folder",
        "Choose which apps can access your downloads folder",
        "",
        ON
    )),
];
/// Privacy & security > Email (`ms-settings:privacy-email`).
pub(super) static EMAIL: [Block; 3] = [
    Block::Card(row!(
        "Email access",
        "Anyone using this device can choose if their apps have access to their email when this is on",
        "g-e119",
        ON
    )),
    Block::Header("Let apps access your email"),
    Block::Card(row!("Choose which apps can access your email", "", "", ON)),
];
/// Privacy & security > Foreground text access (`ms-settings:privacy-foregroundtextaccess`).
pub(super) static FOREGROUND_TEXT_ACCESS: [Block; 2] = [
    Block::Card(row!(
        "Foreground text access",
        "Anyone using this device can choose if their apps can modify text in focusing text box when this is on",
        "g-e7f8",
        ON
    )),
    Block::Card(row!(
        "Let apps access to modify text in focusing text box",
        "Choose which apps can modify text in focusing text box",
        "",
        ON
    )),
];
/// Privacy & security > Location (`ms-settings:privacy-location-spotlightactivity`).
pub(super) static LOCATION: [Block; 7] = [
    Block::Card(row!(
        "Location services",
        "When this is on, location will be available to Doors and anyone using this device can choose whether apps can access their location",
        "g-e37a",
        ON
    )),
    Block::Header("Let apps access your location"),
    Block::Card(row!(
        "All apps can access approximate location, but you can choose which apps can access your precise location",
        "",
        "",
        ON
    )),
    Block::Card(row!(
        "Desktop apps that have previously accessed your location are listed here",
        "",
        "",
        ON
    )),
    Block::Card(row!(
        "Default location",
        "",
        "",
        Control::Button("Set default")
    )),
    Block::Card(row!(
        "Doors, apps, and services can use the location from a remote connection instead of this device's location",
        "",
        "",
        OFF
    )),
    Block::Card(row!(
        "Recent activity",
        "Review location access from the last 7 days. The most recent access data is also captured in other sections in this page",
        "",
        Control::Button("Previous")
    )),
];
/// Privacy & security > Messaging (`ms-settings:privacy-messaging`).
pub(super) static MESSAGING: [Block; 3] = [
    Block::Card(row!(
        "Messaging access",
        "Anyone using this device can choose if their apps have access to read text messages with a cellular connection when this is on",
        "g-e15f",
        ON
    )),
    Block::Header("Let apps read messages"),
    Block::Card(row!("Choose which apps can read messages", "", "", ON)),
];
/// Privacy & security > Music (`ms-settings:privacy-musiclibrary`).
pub(super) static MUSIC: [Block; 3] = [
    Block::Card(row!(
        "Allow access to music libraries on this device",
        "If you allow access, people using this device will be able to choose if their apps have access to their music library by using the settings on this page. If you deny access, all apps that are available in the Macrohard Store on Doors 11 will be blocked from accessing any person's music library.",
        "",
        ON
    )),
    Block::Card(row!(
        "Allow apps to access your music library",
        "If you allow access, you can choose which apps can access your music library by using the settings on this page. If you deny access, apps that are available in the Macrohard Store on Doors 11 will be blocked from accessing your music library.",
        "",
        ON
    )),
    Block::Card(row!(
        "Choose which apps can access your music library",
        "Some apps need to access your music library to work as intended. Turning off an app here might limit what it can do.",
        "",
        ON
    )),
];
/// Privacy & security > Notifications (`ms-settings:privacy-notifications`).
pub(super) static PRIVACY_NOTIFICATIONS: [Block; 2] = [
    Block::Card(row!(
        "Notification access",
        "Anyone using this device can choose if their apps have access to their notifications when this is on",
        "g-ea8f",
        OFF
    )),
    Block::Card(row!(
        "Let apps access your notifications",
        "Choose which apps can access your notifications",
        "",
        OFF
    )),
];
/// Privacy & security > Other devices (`ms-settings:privacy-customdevices`).
pub(super) static OTHER_DEVICES: [Block; 1] = [Block::Card(row!(
    "Communicate with unpaired devices",
    "Let your apps automatically share and sync info with wireless devices that don’t explicitly pair with your PC, tablet, or phone",
    "",
    ON
))];
/// Privacy & security > Passkeys (`ms-settings:privacy-passkeys`).
pub(super) static PRIVACY_PASSKEYS: [Block; 3] = [
    Block::Header("Allow passkey autofill"),
    Block::Card(row!(
        "Passkey autofill access",
        "Anyone using this device can choose if their apps can autofill passkeys when this is on",
        "g-ee7e",
        ON
    )),
    Block::Expander(Expander {
        id: 1,
        header: row!(
            "Let apps autofill passkeys",
            "Choose which apps can autofill passkeys",
            "",
            Control::None
        ),
        hero: None,
        rows: rows![],
    }),
];
/// Privacy & security > Pictures (`ms-settings:privacy-pictures`).
pub(super) static PICTURES: [Block; 3] = [
    Block::Card(row!(
        "Pictures library access",
        "Anyone using this device can choose if their apps have access to their pictures library when this is on",
        "",
        ON
    )),
    Block::Header("Let apps access your pictures library"),
    Block::Card(row!(
        "Choose which apps can access your pictures library",
        "",
        "",
        ON
    )),
];
/// Privacy & security > Radios (`ms-settings:privacy-radios`).
pub(super) static RADIOS: [Block; 2] = [
    Block::Card(row!(
        "Radio control access",
        "Anyone using this device can choose if their apps have access to control device radios when this is on",
        "",
        ON
    )),
    Block::Card(row!(
        "Let apps control device radios",
        "Choose which apps can control your device radios",
        "",
        ON
    )),
];
/// Privacy & security > Screenshot borders (`ms-settings:privacy-graphicscapturewithoutborder`).
pub(super) static SCREENSHOT_BORDERS: [Block; 4] = [
    Block::Card(row!(
        "Screenshot border settings access",
        "Anyone using this device can choose if their apps have access to settings for turning off screenshot borders when this is on",
        "",
        ON
    )),
    Block::Header("Let apps turn off the screenshot border"),
    Block::Card(row!(
        "Choose which apps can access your screenshot border settings",
        "",
        "",
        ON
    )),
    Block::Card(row!(
        "Desktop apps that have previously requested access to your screenshot border settings are listed here",
        "",
        "",
        ON
    )),
];
/// Privacy & security > Screenshots and screen recording (`ms-settings:privacy-graphicscaptureprogrammatic`).
pub(super) static SCREENSHOTS_AND_SCREEN_RECORDING: [Block; 4] = [
    Block::Card(row!(
        "Screenshots and screen recording access",
        "Anyone using this device can choose if their apps can take screenshots or record the screen when this is on",
        "",
        ON
    )),
    Block::Header("Let apps take screenshots and record the screen"),
    Block::Card(row!(
        "Choose which apps can take screenshots and record the screen",
        "",
        "",
        ON
    )),
    Block::Card(row!(
        "Desktop apps that have previously requested access to take screenshots or record the screen are listed here",
        "",
        "",
        ON
    )),
];
/// Privacy & security > Speech (`ms-settings:privacy-speech`).
pub(super) static PRIVACY_SPEECH: [Block; 1] = [Block::Card(row!(
    "Online speech recognition",
    "Use your voice for apps using Macrohard’s online speech recognition technology.",
    "",
    OFF
))];
/// Privacy & security > Tasks (`ms-settings:privacy-tasks`).
pub(super) static TASKS: [Block; 2] = [
    Block::Card(row!(
        "Tasks access",
        "Anyone using this device can choose if their apps have access to their tasks when this is on",
        "g-f7ec",
        ON
    )),
    Block::Card(row!(
        "Let apps access your tasks",
        "Choose which apps can access your tasks",
        "",
        ON
    )),
];
/// Privacy & security > Videos (`ms-settings:privacy-videos`).
pub(super) static VIDEOS: [Block; 3] = [
    Block::Card(row!(
        "Videos library access",
        "Anyone using this device can choose if their apps have access to their videos library when this is on",
        "",
        ON
    )),
    Block::Header("Let apps access your videos library"),
    Block::Card(row!(
        "Choose which apps can access your videos library",
        "",
        "",
        ON
    )),
];
/// Privacy & security > Voice activation (`ms-settings:privacy-voiceactivation`).
pub(super) static VOICE_ACTIVATION: [Block; 3] = [
    Block::Card(row!("Microphone", "On", "", link(Page::Microphone))),
    Block::Expander(Expander {
        id: 1,
        header: row!(
            "Let apps access voice activation services",
            "Let apps use voice activation when device is locked",
            "g-f12e",
            ON
        ),
        hero: None,
        rows: rows![row!(
            "Let apps use voice activation when device is locked",
            "",
            "",
            ON
        )],
    }),
    Block::Card(row!(
        "By default, allow the last used voice activation app to start when headset button is pressed",
        "",
        "",
        NA
    )),
];
/// Privacy & security > Windows Security (`ms-settings:windowsdefender`).
pub(super) static WINDOWS_SECURITY: [Block; 3] = [
    Block::Card(row!(
        "Doors Security",
        "Doors Security is your home to view and manage the security and health of your device.",
        "g-ea18",
        Control::Button("Open Doors Security")
    )),
    Block::Header("Protection areas"),
    Block::Card(row!(
        "Virus & threat protection",
        "No actions needed.",
        "",
        NA
    )),
];
/// System > Activation (`ms-settings:activation`).
pub(super) static ACTIVATION: [Block; 2] = [
    Block::Expander(Expander {
        id: 1,
        header: row!("Activation state", "Active", "g-eb95", Control::None),
        hero: None,
        rows: rows![],
    }),
    Block::Card(row!(
        "Change product key",
        "",
        "g-e192",
        Control::Button("Change")
    )),
];
/// System > Display > Advanced display (`ms-settings:advanceddisplay`).
pub(super) static ADVANCED_DISPLAY: [Block; 3] = [
    Block::Card(row!(
        "Select a display to view or change its settings",
        "",
        "",
        NA
    )),
    Block::Expander(Expander {
        id: 1,
        header: row!("Internal Display", "Display 1", "", Control::None),
        hero: None,
        rows: rows![],
    }),
    Block::Card(row!(
        "Choose a refresh rate",
        "A higher rate gives smoother motion, but also uses more power  More about refresh rate",
        "",
        OFF
    )),
];
/// System > Display > Graphics (`ms-settings:display-advancedgraphics-default`).
pub(super) static GRAPHICS: [Block; 3] = [
    Block::Expander(Expander {
        id: 1,
        header: row!("Default settings", "Informational icon", "", OFF),
        hero: None,
        rows: rows![row!("Hardware-accelerated GPU scheduling", "", "", ON)],
    }),
    Block::Header("Custom settings for applications"),
    Block::Card(row!(
        "Add an app",
        "",
        "",
        Control::Button("Add desktop app")
    )),
];
/// System > Display > HDR (`ms-settings:display-hdr`).
pub(super) static HDR: [Block; 10] = [
    Block::Intro(
        "Get a brighter and more vibrant picture for high dynamic range (HDR) games, videos, and apps. This depends on your PC and display.",
    ),
    Block::Card(row!(
        "Select a display to view or change its settings",
        "",
        "",
        NA
    )),
    Block::Expander(Expander {
        id: 1,
        header: row!(
            "Display capabilities",
            "HDR video streaming, games, apps and more",
            "",
            Control::None
        ),
        hero: None,
        rows: rows![],
    }),
    Block::Card(row!("HDR", "More about HDR", "", OFF)),
    Block::Header("Display calibration for HDR video"),
    Block::Card(row!(
        "Calibrate to preview and adjust HDR video",
        "",
        "",
        Control::Button("Calibrate")
    )),
    Block::Card(row!(
        "Restore default calibration settings",
        "",
        "",
        Control::Button("Restore default")
    )),
    Block::Card(row!(
        "HDR Display Calibration",
        "Improve color accuracy and consistency on your HDR display",
        "",
        NA
    )),
    Block::Header("Related settings"),
    Block::Card(row!(
        "Video playback",
        "Video adjustments, HDR streaming, battery options",
        "",
        link(Page::VideoPlayback)
    )),
];
/// System > Display > Night light (`ms-settings:easeofaccess-colorfilter-bluelightlink`).
pub(super) static NIGHT_LIGHT: [Block; 3] = [
    Block::Card(row!(
        "Show warmer colors on your display to help you sleep",
        "",
        "night",
        Control::Toggle(Toggle::Pref {
            id: p::VISION,
            bit: p::vision::NIGHT_LIGHT
        })
    )),
    Block::Card(row!(
        "Strength",
        "",
        "",
        Control::Combo(Choice::NightStrength)
    )),
    Block::Expander(Expander {
        id: 1,
        header: row!("Schedule night light", "", "", ON),
        hero: None,
        rows: rows![],
    }),
];
/// System > Focus (`ms-settings:quiethours`).
pub(super) static FOCUS: [Block; 4] = [
    Block::Intro(
        "Focus sessions help you get more done by reducing distractions. When you start a session, these settings will apply for a set amount of time.",
    ),
    Block::Expander(Expander {
        id: 1,
        header: row!(
            "Focus",
            "Start a focus session from notification center",
            "eye",
            Control::None
        ),
        hero: None,
        rows: rows![
            row!(
                "Session length",
                "",
                "history",
                Control::Combo(Choice::FocusLength)
            ),
            row!("Show the timer in the Clock app", "", "", NA),
            row!(
                "Hide badges on taskbar apps",
                "",
                "taskbar",
                Control::Toggle(Toggle::Pref {
                    id: p::FOCUS,
                    bit: 1 << 9
                })
            ),
            row!(
                "Hide flashing on taskbar apps",
                "",
                "taskbar",
                Control::Toggle(Toggle::Pref {
                    id: p::FOCUS,
                    bit: 1 << 10
                })
            ),
            row!(
                "Turn on do not disturb",
                "",
                "quiet",
                Control::Toggle(Toggle::Pref {
                    id: p::FOCUS,
                    bit: 1 << 11
                })
            )
        ],
    }),
    Block::Header("Related settings"),
    Block::Card(row!(
        "Notifications",
        "Alerts from apps and system, do not disturb",
        "g-ea8f",
        link(Page::Notifications)
    )),
];
/// System > Optional features (`ms-settings:optionalfeatures`).
pub(super) static OPTIONAL_FEATURES: [Block; 3] = [
    Block::Card(row!(
        "View or edit optional features",
        "",
        "",
        Control::Button("View features")
    )),
    Block::Header("Related settings"),
    Block::Card(row!("More Doors features", "", "", NA)),
];
/// System > Power & battery > Energy recommendations (`ms-settings:energyrecommendations`).
pub(super) static ENERGY_RECOMMENDATIONS: [Block; 7] = [
    Block::Card(row!(
        "Set the screen brightness for best energy efficiency",
        "",
        "",
        Control::Button("Apply")
    )),
    Block::Card(row!(
        "Put my device to sleep after 3 minutes",
        "When you apply this change, we'll also update the timeout setting for your screen",
        "",
        Control::Button("Apply")
    )),
    Block::Card(row!(
        "Turn off my screen after 3 minutes",
        "",
        "",
        Control::Button("Apply")
    )),
    Block::Card(row!("Turn off HDR to conserve energy", "", "", NA)),
    Block::Card(row!(
        "Lower your refresh rate to 60 Hz to conserve energy",
        "Lowering refresh rate will result in a less smooth picture",
        "",
        Control::Button("Apply")
    )),
    Block::Card(row!(
        "Turn off my screen saver",
        "",
        "",
        Control::Button("Apply")
    )),
    Block::Card(row!(
        "Stop USB devices when my screen is off to help save battery",
        "",
        "",
        NA
    )),
];
/// System > Projecting to this PC (`ms-settings:project`).
pub(super) static PROJECTING_TO_THIS_PC: [Block; 1] = [Block::Intro(
    "Project your Doors phone or PC to this screen, and use its keyboard, mouse, and other devices, too.",
)];
/// System > Remote Desktop (`ms-settings:remotedesktop`).
pub(super) static REMOTE_DESKTOP: [Block; 2] = [
    Block::Expander(Expander {
        id: 1,
        header: row!(
            "Remote Desktop",
            "Connect to and use this PC from another device using the Remote Desktop app",
            "g-e148",
            OFF
        ),
        hero: None,
        rows: rows![row!(
            "Require devices to use Network Level Authentication to connect (Recommended)",
            "",
            "",
            NA
        )],
    }),
    Block::Card(row!(
        "Remote Desktop users",
        "Select who can remotely access this PC",
        "g-e181",
        NA
    )),
];
/// System > Share (`ms-settings:crossdevice`).
pub(super) static SHARE: [Block; 3] = [
    Block::Expander(Expander {
        id: 1,
        header: row!(
            "Nearby sharing",
            "Share files, photos, and links with nearby Doors devices",
            "",
            Control::None
        ),
        hero: None,
        rows: rows![],
    }),
    Block::Card(row!(
        "Save files I receive to",
        "Downloads",
        "",
        Control::Button("Change")
    )),
    Block::Card(row!("Show me suggested apps in share surfaces", "", "", ON)),
];
/// System > Sound > All sound devices (`ms-settings:sound-devices`).
pub(super) static ALL_SOUND_DEVICES: [Block; 4] = [
    Block::Header("Output devices"),
    Block::Card(row!(
        "Speakers",
        "Output: VirtIO Sound",
        "volume",
        Control::Value(Fact::Audio(AudioFact::Device))
    )),
    Block::Header("Input devices"),
    Block::Card(row!("Microphone", "", "", link(Page::Microphone))),
];
/// System > Sound > Properties (`ms-settings:sound-defaultinputproperties`).
pub(super) static PROPERTIES: [Block; 5] = [
    Block::Header("General"),
    Block::Header("Input settings"),
    Block::Card(row!("Format", "", "", NA)),
    Block::Card(row!("Input volume", "", "", NA)),
    Block::Expander(Expander {
        id: 1,
        header: row!(
            "Microphone test",
            "Start the test, then talk or play audio at a normal level for a few seconds",
            "",
            NA
        ),
        hero: None,
        rows: rows![],
    }),
];
/// System > Sound > Volume mixer (`ms-settings:apps-volume`).
pub(super) static VOLUME_MIXER: [Block; 7] = [
    Block::Header("System"),
    Block::Card(row!(
        "Volume",
        "",
        "volume",
        Control::Slider(Slider::Volume)
    )),
    Block::Card(row!(
        "Mute all",
        "",
        "",
        Control::Toggle(Toggle::Pref {
            id: p::SOUND,
            bit: 1 << 8
        })
    )),
    Block::Card(row!(
        "Output device",
        "",
        "g-e15d",
        Control::Value(Fact::Audio(AudioFact::Device))
    )),
    Block::Card(row!(
        "Input device",
        "",
        "g-e1d6",
        Control::Value(Fact::Audio(AudioFact::Input))
    )),
    Block::Header("Apps"),
    Block::Card(row!(
        "Reset sound devices and volumes for all apps to the recommended defaults",
        "",
        "",
        Control::Button("Reset")
    )),
];
/// System > Storage > Storage Sense (`ms-settings:storagepolicies`).
pub(super) static STORAGE_SENSE: [Block; 2] = [
    Block::Card(row!("Cleanup of temporary files", "", "", NA)),
    Block::Card(row!(
        "Automatic User content cleanup",
        "Storage Sense runs when disk space is low. We clean up enough space to help your system run its best.",
        "",
        ON
    )),
];
/// System > Storage > Where new content is saved (`ms-settings:savelocations`).
pub(super) static WHERE_NEW_CONTENT_IS_SAVED: [Block; 1] = [Block::Intro(
    "Change where your apps, documents, music, pictures, videos and maps are saved by default.",
)];
/// System > System components (`ms-settings:systemcomponents`).
pub(super) static SYSTEM_COMPONENTS: [Block; 2] = [
    Block::Header("Related settings"),
    Block::Card(row!(
        "Installed apps",
        "Uninstall and manage apps on your PC",
        "",
        link(Page::InstalledApps)
    )),
];
/// Time & language > Language & region > Domain lexicon (`ms-settings:regionlanguage-chsime-pinyin-domainlexicon`).
pub(super) static DOMAIN_LEXICON: [Block; 0] = [];
/// Time & language > Language & region > Keys (`ms-settings:regionlanguage-chsime-pinyin-keyconfig`).
pub(super) static KEYS: [Block; 6] = [
    Block::Card(row!("Chinese/English mode switch", "", "", NA)),
    Block::Card(row!("Full/half width switch", "", "", Control::None)),
    Block::Card(row!(
        "Chinese/English punctuation switch",
        "",
        "",
        Control::None
    )),
    Block::Card(row!(
        "Candidate window switch keys",
        "Default key: PgUp/PgDn; you also can use",
        "",
        NA
    )),
    Block::Card(row!(
        "Character input based on a phrase",
        "",
        "",
        Control::None
    )),
    Block::Card(row!("Hot keys", "", "", ON)),
];
/// Time & language > Language & region > Microsoft IME (`ms-settings:regionlanguage-jpnime`).
pub(super) static MICROSOFT_IME: [Block; 0] = [];
/// Time & language > Language & region > Microsoft Pinyin (`ms-settings:regionlanguage-chsime-pinyin`).
pub(super) static MICROSOFT_PINYIN: [Block; 0] = [];
/// Time & language > Language & region > User defined phrases (`ms-settings:regionlanguage-chsime-pinyin-udp`).
pub(super) static USER_DEFINED_PHRASES: [Block; 0] = [];
/// Time & language > Speech (`ms-settings:speech`).
pub(super) static TIME_SPEECH: [Block; 4] = [
    Block::Intro(
        "Choose the primary language you speak, manage voices for your device and apps, and set up your microphone.",
    ),
    Block::Card(row!("Speech language", "English (United States)", "", NA)),
    Block::Card(row!("Voices", "", "", NA)),
    Block::Card(row!("Manage voices", "", "", Control::Button("Add voices"))),
];
/// Windows Update > Advanced options (`ms-settings:windowsupdate-activehours`).
pub(super) static WINDOWS_UPDATE_ADVANCED_OPTIONS: [Block; 9] = [
    Block::Card(row!(
        "Restart as soon as possible (even during active hours) to finish updating, and notify me 15 minutes before restarting so I can make sure this device is on and plugged in",
        "",
        "",
        OFF
    )),
    Block::Card(row!("Data charges may apply", "", "", OFF)),
    Block::Card(row!(
        "Show notification when your device requires a restart to finish updating",
        "",
        "",
        OFF
    )),
    Block::Expander(Expander {
        id: 1,
        header: row!(
            "Active hours",
            "We won't restart your device during these hours",
            "",
            NA
        ),
        hero: None,
        rows: rows![],
    }),
    Block::Header("Additional options"),
    Block::Card(row!(
        "Optional updates",
        "Feature, quality, and driver updates",
        "g-ecc8",
        link(Page::OptionalUpdates)
    )),
    Block::Card(row!(
        "Recovery",
        "Reset, advanced startup, go back",
        "",
        link(Page::BackupRestore)
    )),
    Block::Card(row!(
        "Restart apps",
        "Automatically save my restartable apps when I sign out and restart them after I sign in",
        "",
        NA
    )),
    Block::Card(row!("Configured update policies", "", "", NA)),
];
/// Windows Update > Optional updates (`ms-settings:windowsupdate-optionalupdates`).
pub(super) static OPTIONAL_UPDATES: [Block; 1] = [Block::Card(row!(
    "Select the updates you want to install",
    "",
    "",
    Control::Button("Download & install")
))];
/// Windows Update > Uninstall updates (`ms-settings:windowsupdate-uninstallupdates`).
pub(super) static UNINSTALL_UPDATES: [Block; 1] = [Block::Note("No updates can be uninstalled.")];
/// Windows Update > Update history (`ms-settings:windowsupdate-history`).
pub(super) static UPDATE_HISTORY: [Block; 3] = [
    Block::Header("Related settings"),
    Block::Card(row!(
        "Uninstall updates",
        "",
        "g-e107",
        link(Page::UninstallUpdates)
    )),
    Block::Card(row!("Recovery", "", "", link(Page::BackupRestore))),
];
