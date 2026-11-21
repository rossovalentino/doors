use super::*;
#[test]
fn native_tab_caption_preserves_scaled_controls_drag_regions_and_owner_boundaries() {
    for scale in 1..=3 {
        let mut e =
            Engine::new(1024 * scale, 720 * scale, Theme::Light, Limits::default()).unwrap();
        e.set_scale(scale).unwrap();
        let owner = Owner {
            pid: 1,
            session: 2,
            epoch: 3,
        };
        let other = Owner { pid: 2, ..owner };
        let r = Rect::new(
            10 * scale as i32,
            20 * scale as i32,
            960 * scale,
            640 * scale,
        )
        .unwrap();
        let id = e
            .create_client_caption(
                owner,
                r,
                CaptionLayout {
                    tabbed: true,
                    height: 56,
                    side_margin: 208 * scale,
                    max_width: 400 * scale,
                },
            )
            .unwrap();
        let point = |x, y| (r.x() + x * scale as i32, r.y() + y * scale as i32);
        for (x, y, region) in [
            (230, 40, HitRegion::Client),
            (800, 28, HitRegion::TitleBar),
            (100, 28, HitRegion::TitleBar),
            (935, 28, HitRegion::Close),
            (890, 28, HitRegion::Maximize),
            (845, 28, HitRegion::Minimize),
            (935, 60, HitRegion::Client),
        ] {
            let (x, y) = point(x, y);
            assert_eq!(e.hit_test(x, y), Some((id, region)));
        }
        assert_eq!(
            e.set_tabbed_caption(other, id, 0, 500),
            Err(Error::WrongOwner)
        );
        assert!(e.set_tabbed_caption(owner, id, 0, 0).is_err());
        e.set_tabbed_caption(owner, id, 8 * scale, 300 * scale)
            .unwrap();
        let (x, y) = point(40, 40);
        assert_eq!(e.hit_test(x, y), Some((id, HitRegion::Client)));
        let (x, y) = point(400, 28);
        assert_eq!(e.hit_test(x, y), Some((id, HitRegion::TitleBar)));
    }
}
#[test]
fn wallpaper_material_and_translucent_caption_match_full_and_partial_composition() {
    use crate::tests::{caption_capture, commit_color};
    let mut e = Engine::new(640, 480, Theme::Light, Limits::default()).unwrap();
    let source: alloc::vec::Vec<u8> = (0..640 * 480).flat_map(|_| [200, 100, 50, 255]).collect();
    e.set_wallpaper_material(&source).unwrap();
    let owner = Owner {
        pid: 1,
        session: 2,
        epoch: 3,
    };
    let id = e
        .create_client_caption(
            owner,
            Rect::new(20, 20, 600, 400).unwrap(),
            CaptionLayout {
                tabbed: true,
                height: 56,
                side_margin: 208,
                max_width: 200,
            },
        )
        .unwrap();
    commit_color(&mut e, owner, id, [100, 100, 100, 200]);
    let (before, _) = caption_capture(&mut e, false);
    let at = (100 * 640 + 100) * 4;
    assert_eq!(&before[at..at + 4], &[143, 122, 111, 255]);
    e.set_caption_pointer(590, 48, false);
    let (hover, _) = caption_capture(&mut e, true);
    assert_eq!(hover, caption_capture(&mut e, false).0);
    e.set_caption_pointer(100, 100, false);
    assert_eq!(before, caption_capture(&mut e, true).0);
    let serial = e.render().unwrap().serial;
    assert_eq!(e.set_wallpaper_material(&source), Err(Error::Busy));
    e.complete_transfer(serial, true, 0).unwrap();
    assert!(e.set_wallpaper_material(&source[..10]).is_err());
    assert_eq!(before, caption_capture(&mut e, false).0);
}
