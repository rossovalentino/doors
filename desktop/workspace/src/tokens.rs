//! Resolved P11 development tokens. Coordinates are 96-DPI logical units on the
//! fixed GOP profile. Published geometry is guidance, not a measured shell pass.
#![allow(dead_code)] // Metadata is retained for the P12 token/gallery exporter.
pub struct Token<T> {
    pub value: T,
    pub unit: &'static str,
    pub profile: &'static str,
    pub source_kind: &'static str,
    pub source: &'static str,
    pub status: &'static str,
    pub controls: &'static str,
}
const fn provisional<T>(value: T, unit: &'static str, controls: &'static str) -> Token<T> {
    Token {
        value,
        unit,
        profile: "p11-light-96dpi",
        source_kind: "development",
        source: "master specification 140.3",
        status: "unmeasured Windows 11 26H2",
        controls,
    }
}
pub const CONTROL_RADIUS: Token<u32> = Token {
    value: 4,
    unit: "effective px",
    profile: "p11-light-96dpi",
    source_kind: "published guidance",
    source: "https://learn.microsoft.com/en-us/windows/apps/design/signature-experiences/geometry",
    status: "unmeasured against selected shell",
    controls: "command buttons",
};
pub const CONTROL_HEIGHT: Token<u32> =
    provisional(30, "effective px", "command buttons and hit targets");
pub const TEXT: Token<[u8; 4]> = provisional(
    [32, 32, 32, 255],
    "premultiplied BGRA8",
    "body and command labels",
);
pub const SUBTLE: Token<[u8; 4]> = provisional(
    [249, 249, 249, 255],
    "premultiplied BGRA8",
    "toolbar and navigation backgrounds",
);
pub const ACCENT: Token<[u8; 4]> = provisional(
    [196, 103, 0, 255],
    "premultiplied BGRA8",
    "primary save command and caret",
);
pub const CONTROL: Token<[u8; 4]> = provisional(
    [253, 253, 253, 255],
    "premultiplied BGRA8",
    "standard command surface",
);
pub const BORDER: Token<[u8; 4]> = provisional(
    [224, 224, 224, 255],
    "premultiplied BGRA8",
    "standard command border",
);
