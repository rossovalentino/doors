//! The Windows sound scheme is Microsoft-owned and copied only into the
//! owner's private build (`assets/sounds/windows-media`, never
//! redistributed). Without those files the service builds without system
//! sounds (PLAY_SOUND answers NotFound) instead of failing.
fn main() {
    let dir =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/sounds/windows-media");
    println!("cargo::rustc-check-cfg=cfg(windows_media)");
    println!("cargo::rerun-if-changed={}", dir.display());
    let all = [
        "background",
        "foreground",
        "notify-system-generic",
        "notify-calendar",
        "notify-email",
        "notify-messaging",
        "unlock",
        "user-account-control",
        "hardware-insert",
        "hardware-remove",
        "hardware-fail",
        "alarm01",
        "message-nudge",
    ]
    .iter()
    .all(|n| dir.join(format!("{n}.wav")).is_file());
    if all {
        println!("cargo::rustc-cfg=windows_media");
    }
}
