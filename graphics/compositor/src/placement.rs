//! Initial placement of a new top-level client window, in logical pixels.
//! The native app runtime requests exactly this rectangle and the authority
//! honours it, so host tools derive input points from the same function.
/// Band kept free below a new window for the (bottom) taskbar.
pub const PLACEMENT_TASKBAR_RESERVE: u32 = 48;
/// Extra windows of one process cascade by this much and are capped in size.
pub const PLACEMENT_CASCADE: i32 = 24;
/// Default size as a share of the work area (per mille), as Windows sizes new
/// windows relative to the screen: 70% of its width, 78% of its height.
pub const PLACEMENT_WIDTH_SHARE: u32 = 700;
pub const PLACEMENT_HEIGHT_SHARE: u32 = 780;
/// The proportional default never shrinks a window below this size (or the
/// work area, if that is smaller).
pub const PLACEMENT_MINIMUM: (u32, u32) = (640, 480);
/// `(x, y, width, height)` for window `index` of a process on an `output`
/// (width, height) output. The work area is the output above the
/// `PLACEMENT_TASKBAR_RESERVE` band. The requested size is capped to 70% x 78%
/// of the work area (rounded), never below 640 x 480 or above the work area,
/// and centred in it; extra windows are capped to 960 x 600 and cascade by
/// `PLACEMENT_CASCADE`. An explicit `position` (tab tear-off) is clamped the
/// same way.
pub fn initial_window(
    output: (u32, u32),
    requested: (u32, u32),
    index: usize,
    position: Option<(i32, i32)>,
) -> (i32, i32, u32, u32) {
    let (ow, oh) = output;
    let work = (ow, oh.saturating_sub(PLACEMENT_TASKBAR_RESERVE));
    let share =
        |extent: u32, share: u32| ((u64::from(extent) * u64::from(share) + 500) / 1000) as u32;
    let cap_w = share(work.0, PLACEMENT_WIDTH_SHARE)
        .max(PLACEMENT_MINIMUM.0)
        .min(work.0);
    let cap_h = share(work.1, PLACEMENT_HEIGHT_SHARE)
        .max(PLACEMENT_MINIMUM.1)
        .min(work.1);
    let width = requested
        .0
        .min(if index > 0 { 960 } else { requested.0 })
        .min(cap_w)
        .max(240.min(ow));
    let height = requested
        .1
        .min(if index > 0 { 600 } else { requested.1 })
        .min(cap_h)
        .max(120.min(oh));
    let offset = index as i32 * PLACEMENT_CASCADE;
    let (x, y) = position.unwrap_or((
        (ow.saturating_sub(width) / 2) as i32 + offset,
        (oh.saturating_sub(height + PLACEMENT_TASKBAR_RESERVE) / 2) as i32 + offset,
    ));
    (
        x.clamp(0, ow.saturating_sub(width) as i32),
        y.clamp(
            0,
            oh.saturating_sub(height + PLACEMENT_TASKBAR_RESERVE) as i32,
        ),
        width,
        height,
    )
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn first_windows_take_a_share_of_the_work_area_and_extra_windows_cascade() {
        // 1280x720 logical (wide 1x and 3840x2160 at 3x): work area 1280x672,
        // so every app (Explorer, Notes, Settings, Terminal, Preview, Task
        // Manager) opens at 896x524, centred above the taskbar.
        for requested in [
            (1000, 640),
            (900, 640),
            (1129, 787),
            (1128, 632),
            (1280, 800),
            (1100, 720),
        ] {
            assert_eq!(
                initial_window((1280, 720), requested, 0, None),
                (192, 74, 896, 524)
            );
        }
        assert_eq!(
            initial_window((1280, 720), (1000, 640), 1, None),
            (216, 98, 896, 524)
        );
        assert_eq!(
            initial_window((1280, 720), (1000, 640), 3, None),
            (264, 146, 896, 524)
        );
        // 1920x1080: 1344x805 caps leave the requested sizes.
        assert_eq!(
            initial_window((1920, 1080), (1000, 640), 0, None),
            (460, 196, 1000, 640)
        );
        assert_eq!(
            initial_window((1920, 1080), (900, 640), 0, None),
            (510, 196, 900, 640)
        );
        assert_eq!(
            initial_window((1920, 1080), (1129, 787), 0, None),
            (395, 122, 1129, 787)
        );
        assert_eq!(
            initial_window((1920, 1080), (1000, 640), 1, None),
            (504, 240, 960, 600)
        );
        // 1024x768: 717x562.
        assert_eq!(
            initial_window((1024, 768), (1000, 640), 0, None),
            (153, 79, 717, 562)
        );
        // 960x540 logical (1920x1080 at 2x): the 640x480 floor.
        assert_eq!(
            initial_window((960, 540), (1000, 640), 0, None),
            (144, 6, 672, 480)
        );
        assert_eq!(
            initial_window((640, 480), (1000, 640), 0, None),
            (0, 0, 640, 432)
        );
        // Smaller requests are never grown; tiny outputs never underflow.
        assert_eq!(
            initial_window((1280, 720), (400, 300), 0, None),
            (440, 186, 400, 300)
        );
        assert_eq!(
            initial_window((200, 100), (1000, 640), 0, None),
            (0, 0, 200, 100)
        );
        // Explicit positions (tab tear-off) are clamped.
        assert_eq!(
            initial_window((1280, 720), (1000, 640), 0, Some((900, -5))),
            (384, 0, 896, 524)
        );
        assert_eq!(
            initial_window((1280, 720), (1000, 640), 1, Some((100, 100))),
            (100, 100, 896, 524)
        );
    }
}
