//! Offline native-size row symbols from the pinned Segoe Fluent font.
pub fn mask(name: &str, scale: u32) -> &'static [u8] {
    macro_rules! glyph {
        ($name:literal) => {
            match scale {
                3 => &door_workspace::asset!(concat!(
                    "../../../../assets/icons/windows-shell/setting-",
                    $name,
                    ".60.mask"
                ))[..],
                2 => &door_workspace::asset!(concat!(
                    "../../../../assets/icons/windows-shell/setting-",
                    $name,
                    ".40.mask"
                ))[..],
                _ => &door_workspace::asset!(concat!(
                    "../../../../assets/icons/windows-shell/setting-",
                    $name,
                    ".20.mask"
                ))[..],
            }
        };
    }
    // Shell glyphs exported without the `setting-` prefix.
    macro_rules! shell {
        ($name:literal) => {
            match scale {
                3 => &door_workspace::asset!(concat!(
                    "../../../../assets/icons/windows-shell/",
                    $name,
                    ".60.mask"
                ))[..],
                2 => &door_workspace::asset!(concat!(
                    "../../../../assets/icons/windows-shell/",
                    $name,
                    ".40.mask"
                ))[..],
                _ => &door_workspace::asset!(concat!(
                    "../../../../assets/icons/windows-shell/",
                    $name,
                    ".20.mask"
                ))[..],
            }
        };
    }
    match name {
        "g-e107" => glyph!("g-e107"),
        "g-e119" => glyph!("g-e119"),
        "g-e125" => glyph!("g-e125"),
        "g-e12e" => glyph!("g-e12e"),
        "g-e147" => glyph!("g-e147"),
        "g-e148" => glyph!("g-e148"),
        "g-e15d" => glyph!("g-e15d"),
        "g-e15f" => glyph!("g-e15f"),
        "g-e181" => glyph!("g-e181"),
        "g-e192" => glyph!("g-e192"),
        "g-e1d6" => glyph!("g-e1d6"),
        "g-e1e3" => glyph!("g-e1e3"),
        "g-e1f6" => glyph!("g-e1f6"),
        "g-e292" => glyph!("g-e292"),
        "g-e37a" => glyph!("g-e37a"),
        "g-e3ba" => glyph!("g-e3ba"),
        "g-e3bb" => glyph!("g-e3bb"),
        "g-e742" => glyph!("g-e742"),
        "g-e771" => glyph!("g-e771"),
        "g-e789" => glyph!("g-e789"),
        "g-e7e8" => glyph!("g-e7e8"),
        "g-e7f8" => glyph!("g-e7f8"),
        "g-e815" => glyph!("g-e815"),
        "g-e81c" => glyph!("g-e81c"),
        "g-e839" => glyph!("g-e839"),
        "g-e962" => glyph!("g-e962"),
        "g-e9ce" => glyph!("g-e9ce"),
        "g-ea1a" => glyph!("g-ea1a"),
        "g-ea6c" => glyph!("g-ea6c"),
        "g-ea8f" => glyph!("g-ea8f"),
        "g-eb95" => glyph!("g-eb95"),
        "g-ebb2" => glyph!("g-ebb2"),
        "g-ec05" => glyph!("g-ec05"),
        "g-ecc8" => glyph!("g-ecc8"),
        "g-eccc" => glyph!("g-eccc"),
        "g-ed4d" => glyph!("g-ed4d"),
        "g-ee56" => glyph!("g-ee56"),
        "g-ee7e" => glyph!("g-ee7e"),
        "g-f12e" => glyph!("g-f12e"),
        "g-f211" => glyph!("g-f211"),
        "g-f2b7" => glyph!("g-f2b7"),
        "g-f4c3" => glyph!("g-f4c3"),
        "g-f73f" => glyph!("g-f73f"),
        "g-f7ec" => glyph!("g-f7ec"),
        "g-e81e" => glyph!("g-e81e"),
        "g-e75a" => glyph!("g-e75a"),
        "g-e722" => glyph!("g-e722"),
        "g-ea18" => glyph!("g-ea18"),
        "g-f67b" => glyph!("g-f67b"),
        "g-e779" => glyph!("g-e779"),
        "g-e787" => glyph!("g-e787"),
        "g-e91b" => glyph!("g-e91b"),
        "g-e717" => glyph!("g-e717"),
        "g-efa9" => glyph!("g-efa9"),
        "g-e8b7" => glyph!("g-e8b7"),
        "g-e118" => glyph!("g-e118"),
        "g-e130" => glyph!("g-e130"),
        "g-e8d6" => glyph!("g-e8d6"),
        "g-e714" => glyph!("g-e714"),
        "g-e9d9" => glyph!("g-e9d9"),
        "g-e772" => glyph!("g-e772"),
        "g-e753" => glyph!("g-e753"),
        "g-f741" => glyph!("g-f741"),
        "g-e2b1" => glyph!("g-e2b1"),
        "g-ee3f" => glyph!("g-ee3f"),
        "g-e1e4" => glyph!("g-e1e4"),
        "g-e185" => glyph!("g-e185"),
        "g-e73e" => glyph!("g-e73e"),
        "g-e734" => glyph!("g-e734"),
        "g-ec92" => glyph!("g-ec92"),
        "g-e8be" => glyph!("g-e8be"),
        "g-ec48" => glyph!("g-ec48"),
        "g-f432" => glyph!("g-f432"),
        "g-e179" => glyph!("g-e179"),
        "text-cursor" => glyph!("text-cursor"),
        "pointer" => glyph!("pointer"),
        "contact" => glyph!("contact"),
        "display" | "desktop" => glyph!("display"),
        "volume" => glyph!("volume"),
        "ringer" => glyph!("ringer"),
        "quiet" | "night" => glyph!("quiet"),
        "power" => glyph!("power"),
        "storage" => glyph!("storage"),
        "share" => glyph!("share"),
        "multitask" => glyph!("multitask"),
        "key" => glyph!("key"),
        "repair" => glyph!("repair"),
        "restore" => glyph!("restore"),
        "remote" => glyph!("remote"),
        "clipboard" => glyph!("clipboard"),
        "info" => glyph!("info"),
        "devices" => glyph!("devices"),
        "print" => glyph!("print"),
        "phone" => glyph!("phone"),
        "camera" => glyph!("camera"),
        "mouse" => glyph!("mouse"),
        "touch" => glyph!("touch"),
        "pen" => glyph!("pen"),
        "play" | "motion" => glyph!("play"),
        "usb" => glyph!("usb"),
        "wifi" => glyph!("wifi"),
        "ethernet" => glyph!("ethernet"),
        "vpn" => glyph!("vpn"),
        "airplane" => glyph!("airplane"),
        "globe" => glyph!("globe"),
        "network" => glyph!("network"),
        // Windows "Installed apps" uses the bulleted list symbol.
        "apps" | "list" => glyph!("apps"),
        "location" => glyph!("location"),
        "link" => glyph!("link"),
        "video" => glyph!("video"),
        "mail" => glyph!("mail"),
        "people" => glyph!("people"),
        "cloud" => glyph!("cloud"),
        "work" => glyph!("work"),
        "calendar" => glyph!("calendar"),
        "world" => glyph!("world"),
        "keyboard" => glyph!("keyboard"),
        "speech" => glyph!("speech"),
        "game" => glyph!("game"),
        "record" => glyph!("record"),
        "font" => glyph!("font"),
        "effects" | "brightness" => glyph!("effects"),
        "search" => glyph!("search"),
        "color" => glyph!("color"),
        "contrast" => glyph!("contrast"),
        "narrator" => glyph!("narrator"),
        "captions" => glyph!("captions"),
        "eye" => glyph!("eye"),
        "shield" => glyph!("shield"),
        "code" => glyph!("code"),
        "settings" => glyph!("settings"),
        "diagnostic" => glyph!("diagnostic"),
        "history" | "pause" => glyph!("history"),
        "microphone" => glyph!("microphone"),
        "personalize" => shell!("personalize"),
        "taskbar" => shell!("system"),
        "start" | "widgets" => shell!("view"),
        "taskview" => shell!("task-view"),
        "refresh" => shell!("refresh"),
        "edit" => shell!("app-edit"),
        "zoom" => shell!("app-zoomin"),
        "resolution" => shell!("app-fit"),
        "rotate" => shell!("app-rotate"),
        "position" => shell!("app-hand"),
        "folder" => shell!("app-folder"),
        "more" => shell!("app-more"),
        "home" => shell!("app-home"),
        _ => &[],
    }
}
/// The 16 px "network not connected" glyph (the shell's export).
pub fn network_off(scale: u32) -> &'static [u8] {
    match scale {
        3 => {
            door_workspace::asset!(
                "../../../../assets/icons/windows-shell/network-disconnected.16-3x.mask"
            )
        }
        2 => door_workspace::asset!(
            "../../../../assets/icons/windows-shell/network-disconnected.32.mask"
        ),
        _ => door_workspace::asset!(
            "../../../../assets/icons/windows-shell/network-disconnected.16.mask"
        ),
    }
}
