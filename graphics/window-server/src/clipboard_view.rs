//! Paints the Win+V clipboard history panel (`clipboard_history` model) and
//! the drag and drop effect label beside the pointer.
extern crate alloc;
use crate::clipboard::Broker;
use crate::clipboard_history::{
    CLEAR_ALL, EMPTY_BODY, EMPTY_TITLE, Geometry, HEIGHT, MORE, OFF_BODY, OFF_TITLE, Panel, TITLE,
    TURN_ON, Target, WIDTH, body_lines, body_pitch, card_height, card_line, preview, state_title,
};
use crate::dnd as dnd_broker;
use crate::paint::{self, Palette};
use crate::taskbar;
use alloc::{string::String, vec::Vec};
use door_graphics_core::{Canvas, Rect};
use door_graphics_text::{FontAtlas, TextStyle};
use door_service_protocol::{clipboard as cb, dnd};
/// Windows opens the panel at the focused app's text caret: below it with
/// left edges aligned, flipped above it when the work area ends first, and
/// kept inside the work area. Without a caret it is right-anchored above
/// the taskbar like the other shell flyouts.
pub fn geometry(layout: taskbar::Layout, panel: &Panel) -> Geometry {
    let Some(caret) = panel.anchor else {
        return Geometry::at(layout.flyout_sized(WIDTH, HEIGHT));
    };
    let a = layout.work_area();
    let w = WIDTH.min(a.width().saturating_sub(24)).max(1);
    let h = HEIGHT.min(a.height().saturating_sub(24)).max(1);
    let (top, bottom) = (a.y() + 12, a.bottom() - 12);
    let below = caret.bottom() + 4;
    let above = caret.y() - 4 - h as i32;
    let y = if below + h as i32 <= bottom {
        below
    } else if above >= top {
        above
    } else {
        bottom - h as i32
    };
    let x = caret.x().min(a.right() - 12 - w as i32).max(a.x() + 12);
    Geometry::at(Rect::new(x, y.max(top), w, h).unwrap())
}
/// Nearest-sample a premultiplied BGRA image into a physical rectangle
/// (aspect kept, centred), clipped to the canvas clip.
fn thumbnail(c: &mut Canvas<'_>, logical: Rect, image: &[u8]) {
    let Ok((w, h)) = cb::image_size(image) else {
        return;
    };
    let Ok(box_) = c.scaled(logical) else { return };
    let (bw, bh) = (box_.width() as u64, box_.height() as u64);
    let (tw, th) = if u64::from(w) * bh > u64::from(h) * bw {
        (bw, (u64::from(h) * bw / u64::from(w)).max(1))
    } else {
        ((u64::from(w) * bh / u64::from(h)).max(1), bh)
    };
    let (x0, y0) = (
        box_.x() + ((bw - tw) / 2) as i32,
        box_.y() + ((bh - th) / 2) as i32,
    );
    let Some(clip) = c.clip_rect() else { return };
    let stride = c.physical_dimensions().2;
    let pixels = c.pixels_mut();
    for ty in 0..th {
        let y = y0 + ty as i32;
        let sy = (ty * u64::from(h) / th) as usize;
        for tx in 0..tw {
            let x = x0 + tx as i32;
            if !clip.contains(x, y) {
                continue;
            }
            let sx = (tx * u64::from(w) / tw) as usize;
            let s = &image[8 + (sy * w as usize + sx) * 4..][..4];
            let at = y as usize * stride + x as usize * 4;
            let inv = 255 - u32::from(s[3]);
            for i in 0..4 {
                pixels[at + i] = (u32::from(s[i]) + (u32::from(pixels[at + i]) * inv + 127) / 255)
                    .min(255) as u8;
            }
        }
    }
}
/// Paint the panel (opaque flyout surface, shadow, header, cards).
pub fn draw(
    c: &mut Canvas<'_>,
    fonts: &FontAtlas,
    panel: &Panel,
    broker: &Broker,
    layout: taskbar::Layout,
    dark: bool,
    pointer: (i32, i32),
) {
    if !panel.open {
        return;
    }
    let palette = Palette::new(dark);
    let g = geometry(layout, panel);
    let p = g.panel;
    paint::shadow(c, p, 8, dark);
    paint::framed(c, p, 8, palette.surface_border, palette.surface);
    let items = broker.history();
    let on = broker.history_enabled();
    let hover = panel.hit(&g, on, items, pointer.0, pointer.1);
    let selected = panel.selected_index(items);
    paint::text_row(
        c,
        fonts,
        p.x() + 16,
        p.y() + 12,
        paint::text_grow(28, TextStyle::Semibold14),
        TITLE,
        TextStyle::Semibold14,
        palette.text,
        (g.clear.x() - p.x() - 24).max(1) as u32,
    );
    if !on {
        paint::glyph_in(
            c,
            Rect::new(p.x(), p.y() + 96, p.width(), 40).unwrap(),
            20,
            crate::shell_mask!("setting-clipboard", 20),
            palette.text_secondary,
        );
        let (title, body) = state_title(p, 140);
        paint::text_center(
            c,
            fonts,
            p.x() + WIDTH as i32 / 2,
            title.y(),
            title.height(),
            OFF_TITLE,
            TextStyle::Semibold14,
            palette.text,
            WIDTH - 48,
        );
        wrapped(
            c,
            fonts,
            Rect::new(p.x() + 32, body, WIDTH - 64, 44).unwrap(),
            OFF_BODY,
            palette.text_secondary,
        );
        let b = g.turn_on();
        let fill = if hover == Some(Target::TurnOn) {
            paint::alpha(palette.accent, 230)
        } else {
            palette.accent
        };
        paint::rounded(c, b, 4, fill);
        paint::text_in(
            c,
            fonts,
            b,
            TURN_ON,
            TextStyle::Regular14,
            palette.accent_text,
        );
        return;
    }
    if items.is_empty() {
        let (title, body) = state_title(p, 150);
        paint::text_center(
            c,
            fonts,
            p.x() + WIDTH as i32 / 2,
            title.y(),
            title.height(),
            EMPTY_TITLE,
            TextStyle::Semibold14,
            palette.text,
            WIDTH - 48,
        );
        wrapped(
            c,
            fonts,
            Rect::new(p.x() + 32, body, WIDTH - 64, 44).unwrap(),
            EMPTY_BODY,
            palette.text_secondary,
        );
        return;
    }
    if hover == Some(Target::ClearAll) {
        paint::rounded(c, g.clear, 4, palette.subtle_hover);
    }
    paint::text_in(
        c,
        fonts,
        g.clear,
        CLEAR_ALL,
        TextStyle::Regular14,
        palette.accent,
    );
    let saved = c.clip_rect();
    if let (Some(s), Ok(list)) = (saved, c.scaled(g.list))
        && let Some(clip) = s.intersection(list)
    {
        c.set_clip(clip);
        for (i, item) in items.iter().enumerate() {
            let card = g.card(i, panel.scroll);
            if card.bottom() < g.list.y() || card.y() > g.list.bottom() {
                continue;
            }
            let hot = matches!(hover, Some(Target::Item(h)) if h == item.id);
            paint::framed(
                c,
                card,
                6,
                palette.tile_border,
                if hot { palette.tile } else { palette.card },
            );
            if selected == i {
                paint::focus_ring(c, card, &palette);
            }
            let text_w = card.width() - 24 - MORE;
            if let Some(image) = item.content.get(cb::IMAGE) {
                thumbnail(
                    c,
                    Rect::new(card.x() + 12, card.y() + 10, text_w, card_height() - 20).unwrap(),
                    image,
                );
            } else {
                for (row, line) in preview(item).iter().enumerate() {
                    let fitted = paint::ellipsize(fonts, line, TextStyle::Regular14, text_w);
                    let line = card_line();
                    paint::text_row(
                        c,
                        fonts,
                        card.x() + 12,
                        card.y() + 12 + row as i32 * line as i32,
                        line,
                        &fitted,
                        TextStyle::Regular14,
                        palette.text,
                        text_w,
                    );
                }
            }
            let more = g.more(i, panel.scroll);
            if matches!(hover, Some(Target::More(h)) if h == item.id) || panel.menu == Some(item.id)
            {
                paint::rounded(c, more, 4, palette.subtle_hover);
            }
            paint::glyph_in(
                c,
                more,
                16,
                crate::shell_mask!("explorer-more", 16),
                palette.text,
            );
            if item.pinned {
                paint::glyph(
                    c,
                    more.x() + 8,
                    more.bottom() + 8,
                    16,
                    crate::shell_mask!("shell-pin", 16),
                    palette.text_secondary,
                );
            }
        }
    }
    if let Some(s) = saved {
        c.set_clip(s);
    }
    if let Some(i) = panel.menu_index(items) {
        let m = g.menu(i, panel.scroll);
        paint::shadow(c, m, 8, dark);
        paint::framed(c, m, 8, palette.menu_border, palette.menu);
        let rows = [
            (
                Target::Pin,
                if items[i].pinned { "Unpin" } else { "Pin" },
                crate::shell_mask!("shell-pin", 16),
            ),
            (
                Target::Delete,
                "Delete",
                crate::shell_mask!("explorer-delete", 16),
            ),
        ];
        for (row, (target, label, mask)) in rows.into_iter().enumerate() {
            let r = g.menu_row(i, panel.scroll, row as i32);
            if hover == Some(target) {
                paint::rounded(c, r, 4, palette.subtle_hover);
            }
            paint::glyph(
                c,
                r.x() + 12,
                r.y() + (r.height() as i32 - 16) / 2,
                16,
                mask,
                palette.text,
            );
            paint::text_row(
                c,
                fonts,
                r.x() + 40,
                r.y(),
                r.height(),
                label,
                TextStyle::Regular14,
                palette.text,
                r.width() - 48,
            );
        }
    }
}
/// `text` broken at spaces into Regular12 lines at most `width` wide.
pub fn wrap_lines(fonts: &FontAtlas, text: &str, width: u32) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    let mut line = String::new();
    for word in text.split(' ') {
        let candidate = if line.is_empty() {
            String::from(word)
        } else {
            alloc::format!("{line} {word}")
        };
        if paint::measure(fonts, &candidate, TextStyle::Regular12) > width && !line.is_empty() {
            lines.push(core::mem::replace(&mut line, String::from(word)));
        } else {
            line = candidate;
        }
    }
    lines.push(line);
    lines
}
/// Up to `body_lines()` centred lines, broken at spaces.
fn wrapped(c: &mut Canvas<'_>, fonts: &FontAtlas, r: Rect, text: &str, color: [u8; 4]) {
    let lines = wrap_lines(fonts, text, r.width());
    let pitch = body_pitch();
    for (i, l) in lines.iter().take(body_lines() as usize).enumerate() {
        paint::text_center(
            c,
            fonts,
            r.x() + r.width() as i32 / 2,
            r.y() + i as i32 * pitch as i32,
            pitch,
            l,
            TextStyle::Regular12,
            color,
            r.width(),
        );
    }
}
/// Drag feedback: the label Windows shows beside the pointer. With the
/// target's description insert it reads "Copy to Documents", "Move to
/// Documents" or "Create link in Documents" (the Explorer DROPDESCRIPTION
/// strings); without one, the bare effect. None while the drop would be
/// refused (the pointer shows "can't drop" alone).
pub fn feedback_label(feedback: &dnd_broker::Feedback) -> Option<(String, crate::paint::Mask)> {
    let (verb, preposition, mask) = match feedback.effect {
        dnd::COPY => ("Copy", "to", crate::shell_mask!("explorer-copy", 16)),
        dnd::MOVE => ("Move", "to", crate::shell_mask!("explorer-forward", 16)),
        dnd::LINK => ("Create link", "in", crate::shell_mask!("explorer-link", 16)),
        _ => return None,
    };
    let label = match feedback
        .insert
        .as_ref()
        .map(|i| i.as_str())
        .filter(|s| !s.is_empty())
    {
        Some(insert) => alloc::format!("{verb} {preposition} {insert}"),
        None => String::from(verb),
    };
    Some((label, mask))
}
/// Widest drag label (longer descriptions are ellipsized).
const FEEDBACK_MAX: u32 = 320;
/// Logical bounds of the effect label at the pointer (kept inside `bounds`).
pub fn feedback_rect(
    fonts: &FontAtlas,
    feedback: &dnd_broker::Feedback,
    pointer: (i32, i32),
    bounds: Rect,
) -> Option<Rect> {
    let (label, _) = feedback_label(feedback)?;
    let w = (40 + paint::measure(fonts, &label, TextStyle::Regular12))
        .min(paint::text_len(FEEDBACK_MAX));
    let h = paint::text_grow_i(28, TextStyle::Regular12);
    let x = (pointer.0 + 16)
        .min(bounds.right() - w as i32)
        .max(bounds.x());
    let y = (pointer.1 + 22).min(bounds.bottom() - h).max(bounds.y());
    Rect::new(x, y, w, h as u32).ok()
}
/// Paint the effect label (a Windows 11 tooltip-style plate).
pub fn draw_feedback(
    c: &mut Canvas<'_>,
    fonts: &FontAtlas,
    feedback: &dnd_broker::Feedback,
    r: Rect,
    dark: bool,
) {
    let Some((label, mask)) = feedback_label(feedback) else {
        return;
    };
    let palette = Palette::new(dark);
    paint::shadow(c, r, 4, dark);
    paint::framed(c, r, 4, palette.menu_border, palette.menu);
    paint::glyph(
        c,
        r.x() + 10,
        r.y() + (r.height() as i32 - 16) / 2,
        16,
        mask,
        palette.accent,
    );
    let room = r.width().saturating_sub(40);
    let fitted = paint::ellipsize(fonts, &label, TextStyle::Regular12, room);
    paint::text_row(
        c,
        fonts,
        r.x() + 32,
        r.y(),
        r.height(),
        &fitted,
        TextStyle::Regular12,
        palette.text,
        room,
    );
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::clipboard_history::tests::broker_with;
    #[test]
    fn panel_sits_above_the_taskbar_in_the_work_area() {
        let none = Panel::default();
        for (w, h) in [(1280, 720), (1920, 1080), (1024, 768)] {
            let layout = taskbar::Layout::new(w, h);
            let g = geometry(layout, &none);
            assert_eq!((g.panel.width(), g.panel.height()), (WIDTH, HEIGHT));
            assert_eq!(layout.work_area().intersection(g.panel), Some(g.panel));
            assert!(g.panel.intersection(layout.bar()).is_none());
        }
        // The model tests' sample is this placement.
        let g = geometry(taskbar::Layout::new(1280, 720), &none);
        assert_eq!(g.panel, crate::clipboard_history::tests::sample().panel);
    }
    #[test]
    fn panel_opens_at_the_text_caret_and_flips_above_it_near_the_bottom() {
        let layout = taskbar::Layout::new(1280, 720);
        let work = layout.work_area();
        let mut p = Panel::default();
        // Caret near the top left: below it, left edges aligned.
        p.show_at(Some(Rect::new(200, 100, 1, 18).unwrap()));
        let g = geometry(layout, &p);
        assert_eq!((g.panel.x(), g.panel.y()), (200, 122));
        // Near the bottom: above the caret.
        p.show_at(Some(Rect::new(200, 600, 1, 18).unwrap()));
        let g = geometry(layout, &p);
        assert_eq!(g.panel.bottom(), 596);
        // Near the right edge: kept inside the work area.
        p.show_at(Some(Rect::new(1270, 100, 1, 18).unwrap()));
        let g = geometry(layout, &p);
        assert_eq!(work.intersection(g.panel), Some(g.panel));
        assert_eq!(g.panel.right(), work.right() - 12);
    }
    #[test]
    fn drag_labels_name_the_effect_and_target_and_stay_on_screen() {
        let fonts = FontAtlas::windows_reference_scaled(1).unwrap();
        let out = Rect::new(0, 0, 1280, 720).unwrap();
        let fb = |effect, insert: Option<&str>| dnd_broker::Feedback {
            effect,
            taskbar: false,
            insert: insert.and_then(dnd_broker::Insert::new),
        };
        assert!(
            feedback_rect(&fonts, &fb(dnd::NONE, Some("Documents")), (10, 10), out).is_none(),
            "nothing for a refused drop"
        );
        for effect in [dnd::COPY, dnd::MOVE, dnd::LINK] {
            let r = feedback_rect(
                &fonts,
                &fb(effect, Some("A very long folder name that keeps going on")),
                (1270, 710),
                out,
            )
            .unwrap();
            assert_eq!(out.intersection(r), Some(r));
            assert!(r.width() <= FEEDBACK_MAX);
        }
        assert_eq!(
            feedback_label(&fb(dnd::COPY, Some("Documents"))).unwrap().0,
            "Copy to Documents"
        );
        assert_eq!(
            feedback_label(&fb(dnd::MOVE, Some("文档"))).unwrap().0,
            "Move to 文档"
        );
        assert_eq!(
            feedback_label(&fb(dnd::LINK, Some("Desktop"))).unwrap().0,
            "Create link in Desktop"
        );
        assert_eq!(feedback_label(&fb(dnd::MOVE, None)).unwrap().0, "Move");
    }
    #[test]
    fn panel_paints_every_state_inside_its_bounds() {
        let layout = taskbar::Layout::new(1280, 720);
        let fonts = FontAtlas::windows_reference_scaled(1).unwrap();
        let g = geometry(layout, &Panel::default());
        let mut p = Panel::default();
        p.show();
        for (broker, menu) in [
            (Broker::new(), None),
            (broker_with(&[]), None),
            (broker_with(&["Hello", "World", "Three"]), Some(1)),
        ] {
            p.menu = menu;
            let mut bytes = alloc::vec![0u8; 1280 * 720 * 4];
            for px in bytes.chunks_exact_mut(4) {
                px.copy_from_slice(&[40, 80, 120, 255]);
            }
            let before = bytes.clone();
            let mut c = Canvas::new(&mut bytes, 1280, 720, 1280 * 4).unwrap();
            draw(&mut c, &fonts, &p, &broker, layout, false, (0, 0));
            let grown = paint::grow(g.panel, 26, layout.output());
            let mut inside = 0;
            for (i, (a, b)) in bytes
                .chunks_exact(4)
                .zip(before.chunks_exact(4))
                .enumerate()
            {
                if a != b {
                    let (x, y) = ((i % 1280) as i32, (i / 1280) as i32);
                    assert!(grown.contains(x, y), "painted outside the panel at {x},{y}");
                    inside += 1;
                }
            }
            assert!(inside > (WIDTH * HEIGHT) as usize / 2);
        }
    }
}
