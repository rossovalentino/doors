//! Toolkit host tests: routing, the button state machine, focus scopes and
//! restoration, layers and popups, async generations, damage accounting and
//! semantics. Catalog-wide keyboard, semantic-tree and golden-image checks
//! live in the component gallery (`desktop/component-gallery`).
use crate::controls::*;
use crate::*;
use alloc::boxed::Box;
use door_accessibility::{Action, Role, States};
use std::vec;

fn ui() -> Ui {
    Ui::new(
        LSize::px(640, 480),
        Theme::light(),
        Box::new(AtlasText::windows(100).unwrap()),
    )
}
fn press(ui: &mut Ui, id: WidgetId) {
    let c = ui.rect(id).unwrap().center();
    ui.pointer_move(c, Modifiers::NONE);
    ui.pointer_down(c, MouseButton::Primary, Modifiers::NONE);
}
fn release_at(ui: &mut Ui, p: LPoint) {
    ui.pointer_move(p, Modifiers::NONE);
    ui.pointer_up(p, MouseButton::Primary, Modifiers::NONE);
}
fn kinds(ui: &mut Ui) -> std::vec::Vec<Notify> {
    ui.take_notifications()
        .into_iter()
        .map(|n| n.kind)
        .collect()
}

#[test]
fn button_state_machine_follows_the_activation_rules() {
    let mut ui = ui();
    let col = ui.add(ui.root(), Panel::column(8).padding(Insets::all(20)));
    let b = ui.add(col, Button::new("Save"));
    let other = ui.add(col, Button::new("Other"));
    ui.update();
    let inside = ui.rect(b).unwrap().center();
    // Press, release inside: invoke once; focus moved on press, no visual.
    press(&mut ui, b);
    assert_eq!(ui.focused(), Some(b));
    assert!(!ui.focus_visible());
    release_at(&mut ui, inside);
    assert_eq!(kinds(&mut ui), [Notify::Invoked]);
    // Press, drag out, release outside: nothing.
    press(&mut ui, b);
    let outside = ui.rect(other).unwrap().center();
    release_at(&mut ui, outside);
    assert!(kinds(&mut ui).is_empty());
    // Press then lost capture (window deactivation): cancelled.
    press(&mut ui, b);
    ui.window_focus(false);
    ui.window_focus(true);
    release_at(&mut ui, inside);
    assert!(kinds(&mut ui).is_empty());
    // Keyboard: Space invokes on release, Enter on press, Escape cancels.
    ui.key_down(KeyEvent::plain(Key::Space));
    assert!(kinds(&mut ui).is_empty());
    ui.key_up(KeyEvent::plain(Key::Space));
    assert_eq!(kinds(&mut ui), [Notify::Invoked]);
    ui.key_down(KeyEvent::plain(Key::Enter));
    assert_eq!(kinds(&mut ui), [Notify::Invoked]);
    ui.key_down(KeyEvent::plain(Key::Space));
    ui.key_down(KeyEvent::plain(Key::Escape));
    ui.key_up(KeyEvent::plain(Key::Space));
    assert!(kinds(&mut ui).is_empty());
    // Disabled controls never invoke, by pointer, keyboard or automation.
    ui.set_enabled(b, false);
    assert_ne!(ui.focused(), Some(b), "focus leaves a disabled control");
    press(&mut ui, b);
    release_at(&mut ui, inside);
    let node = widget_node(b);
    assert_eq!(
        ui.accessibility_action(node, Action::Invoke),
        Err(ActionError::NotAccepted)
    );
    assert!(kinds(&mut ui).is_empty());
}

#[test]
fn tab_order_scopes_modality_and_restoration() {
    let mut ui = ui();
    let col = ui.add(ui.root(), Panel::column(8));
    let a = ui.add(col, Button::new("A"));
    let b = ui.add(col, CheckBox::new("B"));
    let hidden = ui.add(col, Button::new("Hidden"));
    ui.set_visible(hidden, false);
    let c = ui.add(col, TextBox::new("C"));
    ui.set_tab_index(c, Some(0));
    ui.update();
    assert_eq!(
        ui.tab_order(),
        [c, a, b],
        "explicit index first, hidden skipped"
    );
    ui.key(Key::Tab, Modifiers::NONE);
    assert_eq!(ui.focused(), Some(c));
    assert!(ui.focus_visible(), "keyboard focus shows the visual");
    ui.key(Key::Tab, Modifiers::SHIFT);
    assert_eq!(ui.focused(), Some(b), "wraps backwards");
    // A modal dialog traps Tab and restores focus when it closes.
    let d = ContentDialog::build(&mut ui, "Title", "Body", Some("OK"), None, "Cancel");
    ContentDialog::show(&mut ui, &d);
    ui.update();
    let inside = ui.tab_order();
    assert_eq!(inside.len(), 2);
    assert!(inside.contains(&ui.focused().unwrap()));
    for _ in 0..5 {
        ui.key(Key::Tab, Modifiers::NONE);
        assert!(
            inside.contains(&ui.focused().unwrap()),
            "focus never escapes the dialog"
        );
    }
    // Clicking outside a modal does nothing.
    ui.pointer_down(LPoint::px(2, 2), MouseButton::Primary, Modifiers::NONE);
    assert!(ui.layer_open(d.root));
    ui.key(Key::Escape, Modifiers::NONE);
    assert!(!ui.layer_open(d.root));
    assert_eq!(
        ui.take_notifications().last().unwrap().kind,
        Notify::Closed(RESULT_NONE)
    );
    assert_eq!(ui.focused(), Some(b), "restored");
    // Removing the focused widget moves focus to the next tab stop.
    ui.remove(b);
    assert_eq!(
        ui.focused(),
        Some(c),
        "next stop after the removed one wraps to the first"
    );
    // Window deactivation remembers focus.
    ui.window_focus(false);
    ui.window_focus(true);
    assert_eq!(ui.focused(), Some(c));
}

#[test]
fn popups_take_input_first_and_light_dismiss() {
    let mut ui = ui();
    // The WinUI drop-down opens over its field (the selected row lies on the
    // field), so the other button sits beside the combo, clear of the list.
    let row = ui.add(ui.root(), Panel::row(200).padding(Insets::all(20)));
    ui.style_mut(row, |s| s.h_align = Align::Start);
    let combo = ui.add(
        row,
        ComboBox::new("Theme", &["Light", "Dark", "System"]).selected(0),
    );
    let after = ui.add(row, Button::new("After"));
    ui.update();
    ui.focus(combo);
    ui.key(Key::Down, Modifiers::ALT);
    assert!(ui.widget::<ComboBox>(combo).unwrap().is_open());
    assert_eq!(ui.overlays(), [combo]);
    ui.key(Key::Down, Modifiers::NONE);
    ui.key(Key::Enter, Modifiers::NONE);
    assert_eq!(ui.widget::<ComboBox>(combo).unwrap().selected, Some(1));
    assert!(ui.overlays().is_empty());
    assert_eq!(ui.focused(), Some(combo), "focus stayed on the owner");
    // Closed: arrows change the selection directly; type-ahead works.
    ui.key(Key::Char('s'), Modifiers::NONE);
    assert_eq!(ui.widget::<ComboBox>(combo).unwrap().selected, Some(2));
    // A click outside an open drop-down dismisses it and is consumed.
    ui.key(Key::F(4), Modifiers::NONE);
    let popup = ui.widget::<ComboBox>(combo).unwrap().overlay().unwrap();
    assert!(
        !popup.contains(ui.rect(after).unwrap().center()),
        "{popup:?}"
    );
    press(&mut ui, after);
    assert!(!ui.widget::<ComboBox>(combo).unwrap().is_open());
    assert_eq!(
        ui.focused(),
        Some(combo),
        "the dismissing click did not reach the button"
    );
    // Escape dismisses too; resizing closes popups.
    ui.key(Key::F(4), Modifiers::NONE);
    ui.key(Key::Escape, Modifiers::NONE);
    assert!(ui.overlays().is_empty());
    ui.key(Key::F(4), Modifiers::NONE);
    ui.resize(LSize::px(600, 400));
    assert!(ui.overlays().is_empty());
}

#[test]
fn async_results_are_tagged_by_widget_generation() {
    struct Loader(std::vec::Vec<u32>);
    impl Widget for Loader {
        fn type_name(&self) -> &'static str {
            "Loader"
        }
        fn paint(&self, _: &mut PaintCx<'_>) {}
        fn async_result(&mut self, cx: &mut EventCx<'_>, p: Box<dyn core::any::Any>) {
            self.0.push(*p.downcast::<u32>().unwrap());
            cx.invalidate();
        }
    }
    let mut ui = ui();
    let w = ui.add(ui.root(), Loader(vec![]));
    let stale = ui.begin_async(w).unwrap();
    let fresh = ui.begin_async(w).unwrap();
    assert!(!ui.deliver(stale, Box::new(1u32)), "superseded operation");
    assert!(ui.deliver(fresh, Box::new(2u32)));
    assert_eq!(ui.widget::<Loader>(w).unwrap().0, [2]);
    let late = ui.begin_async(w).unwrap();
    ui.remove(w);
    let recycled = ui.add(ui.root(), Loader(vec![]));
    assert_eq!(recycled.index(), w.index(), "slot reused");
    assert!(
        !ui.deliver(late, Box::new(3u32)),
        "a closed widget never receives a late result"
    );
    assert!(ui.widget::<Loader>(recycled).unwrap().0.is_empty());
    assert_eq!(ui.stats().async_discarded, 2);
}

#[test]
fn damage_covers_exactly_what_changed() {
    let mut ui = ui();
    let col = ui.add(ui.root(), Panel::column(8).padding(Insets::all(20)));
    let a = ui.add(col, Button::new("Alpha"));
    let b = ui.add(col, Button::new("Beta"));
    ui.update();
    let mut pixels = vec![0u8; 640 * 480 * 4];
    let mut fonts = Fonts::new();
    let full = ui.render(
        &mut Frame::new(&mut pixels, 640, 480, 640 * 4).unwrap(),
        door_graphics_core::Scale::ONE,
        &mut fonts,
        true,
    );
    assert_eq!(full.damage_pixels, 640 * 480);
    // Nothing changed: no damage, nothing redrawn.
    ui.update();
    assert!(ui.damage().is_none());
    // Hover one button: damage is that button only.
    ui.pointer_move(ui.rect(b).unwrap().center(), Modifiers::NONE);
    ui.update();
    let d = ui.damage().unwrap();
    assert_eq!(d, ui.rect(b).unwrap());
    assert!(!d.intersects(ui.rect(a).unwrap()));
    assert_eq!(ui.stats().damaged_widgets_last, 1);
    let partial = ui.render(
        &mut Frame::new(&mut pixels, 640, 480, 640 * 4).unwrap(),
        door_graphics_core::Scale::ONE,
        &mut fonts,
        false,
    );
    assert!(partial.damage_pixels < 640 * 48 && partial.skipped > 0);
    // Keyboard focus adds the ring outside the bounds.
    ui.focus(a);
    ui.key(Key::Tab, Modifiers::NONE);
    ui.update();
    let ring = ui.damage().unwrap();
    assert!(
        ring.w > ui.rect(b).unwrap().w,
        "focus visual extends past the control"
    );
    // Inspector reports reasons and cost.
    let i = ui.inspect();
    let node = i.nodes.iter().find(|n| n.id == b).unwrap();
    assert!(node.reasons.names().contains(&"focus"));
    assert!(node.paint_ops > 0);
    assert!(i.text().contains("Button #"));
}

#[test]
fn semantics_publish_states_values_actions_and_deltas() {
    let mut ui = ui();
    let col = ui.add(ui.root(), Panel::column(8));
    let s = ui.add(col, ToggleSwitch::new("Wi-Fi"));
    let t = ui.add(col, TextBox::new("Name").text("Ada"));
    let p = ui.add(col, TextBox::password("Password").text("secret"));
    let e = ui.add(col, Expander::new("More"));
    let inner = ui.add(e, CheckBox::new("Inner option"));
    ui.set_title("Test");
    let first = ui.take_semantics().unwrap();
    assert!(first.delta.is_none());
    let tree = ui.semantics().unwrap().clone();
    assert_eq!(tree.root().role, Role::Window);
    let sw = tree.get(widget_node(s)).unwrap();
    assert!(sw.is(States::CHECKABLE) && !sw.is(States::CHECKED));
    assert_eq!(tree.get(widget_node(t)).unwrap().value, "Ada");
    let pw = tree.get(widget_node(p)).unwrap();
    assert!(
        pw.is(States::PROTECTED) && pw.value.is_empty(),
        "password content is never published"
    );
    assert!(
        tree.get(widget_node(inner)).is_none(),
        "collapsed content is not published"
    );
    // Actions through the semantic tree.
    ui.accessibility_action(widget_node(s), Action::Toggle)
        .unwrap();
    assert!(ui.widget::<ToggleSwitch>(s).unwrap().on);
    ui.accessibility_action(widget_node(t), Action::SetValue("Grace".into()))
        .unwrap();
    assert_eq!(ui.widget::<TextBox>(t).unwrap().value(), "Grace");
    ui.accessibility_action(widget_node(e), Action::Expand)
        .unwrap();
    assert!(ui.semantics().unwrap().get(widget_node(inner)).is_some());
    assert_eq!(
        ui.accessibility_action(widget_node(e), Action::Expand),
        Err(ActionError::NotAccepted)
    );
    let update = ui.take_semantics().unwrap();
    let delta = update.delta.expect("small changes publish a delta");
    assert!(delta.len() < update.full.len());
    // Versions in between were folded into one update whose delta applies to
    // the tree the consumer last took, and reproduces the current tree.
    assert_eq!(update.base, Some(first.version));
    assert!(
        update.version > first.version + 1,
        "several versions folded"
    );
    let mut mirror = door_accessibility::mirror::Mirror::default();
    mirror.apply_bytes(&first.full).unwrap();
    assert_eq!(
        mirror.apply_bytes(&delta).unwrap(),
        door_accessibility::mirror::Applied::Current(update.version)
    );
    assert_eq!(mirror.tree(1), ui.semantics());
    assert!(ui.take_semantics().is_none(), "nothing new");
}

#[test]
fn scroll_containers_clip_hit_test_and_reveal_focus() {
    let mut ui = ui();
    let sv = ui.add(ui.root(), ScrollViewer::vertical());
    let col = ui.add(sv, Panel::column(0));
    let buttons: std::vec::Vec<WidgetId> = (0..30)
        .map(|i| ui.add(col, Button::new(&std::format!("Item {i}"))))
        .collect();
    ui.update();
    let last = *buttons.last().unwrap();
    assert!(ui.rect(last).unwrap().y > Lu::px(480));
    assert_ne!(ui.hit_test(LPoint::px(20, 470)), Some(last));
    ui.focus(buttons[0]);
    ui.key(Key::Tab, Modifiers::SHIFT);
    assert_eq!(ui.focused(), Some(last));
    ui.update();
    let r = ui.rect(last).unwrap();
    assert!(
        r.bottom() <= Lu::px(480) && r.y >= Lu::ZERO,
        "keyboard focus scrolled into view: {r:?} scroll={:?} extent={:?}",
        ui.scroll_offset(sv),
        ui.scroll_extent(sv)
    );
    assert_eq!(ui.hit_test(r.center()), Some(last));
    // Wheel over content scrolls the viewer.
    let before = ui.scroll_offset(sv);
    ui.wheel(LPoint::px(20, 200), 2, false, Modifiers::NONE);
    assert_eq!(ui.scroll_offset(sv).y, before.y - Lu::px(96));
    let tree = ui.semantics().unwrap();
    let first = tree.get(widget_node(buttons[0])).unwrap();
    assert!(first.is(States::OFFSCREEN));
}

#[test]
fn tooltips_follow_hover_delay_and_reduced_motion_stops_ticking() {
    let mut ui = ui();
    let col = ui.add(ui.root(), Panel::column(8));
    let b = ui.add(col, Button::new("Tip"));
    ui.set_tooltip(b, "Saves the file");
    ui.add(col, ProgressBar::new("Busy", 0).indeterminate());
    ui.update();
    ui.set_time(1000);
    ui.pointer_move(ui.rect(b).unwrap().center(), Modifiers::NONE);
    assert!(ui.wants_tick());
    ui.tick(1100);
    assert!(ui.tooltip_shown().is_none());
    ui.tick(1500);
    assert_eq!(ui.tooltip_shown(), Some(b));
    assert!(
        ui.semantics()
            .unwrap()
            .nodes()
            .any(|n| n.role == Role::ToolTip && n.name == "Saves the file")
    );
    ui.pointer_leave();
    assert!(ui.tooltip_shown().is_none());
    ui.update();
    assert!(ui.wants_tick(), "indeterminate progress animates");
    ui.set_theme(Theme::light().with_reduced_motion(true));
    ui.tick(2000);
    ui.update();
    assert!(
        !ui.wants_tick(),
        "reduced motion leaves no idle animation timers"
    );
}

fn click(ui: &mut Ui, id: WidgetId) {
    let c = ui.rect(id).unwrap().center();
    press(ui, id);
    release_at(ui, c);
}
fn node_of(ui: &mut Ui, id: WidgetId) -> door_accessibility::Node {
    ui.semantics()
        .unwrap()
        .get(widget_node(id))
        .unwrap()
        .clone()
}

#[test]
fn toggle_and_selection_controls_follow_their_state_machines() {
    let mut ui = ui();
    let col = ui.add(ui.root(), Panel::column(8).padding(Insets::all(20)));
    let cb = ui.add(col, CheckBox::new("Two state"));
    let tri = ui.add(col, CheckBox::new("Three state").three_state());
    let sw = ui.add(col, ToggleSwitch::new("Wi-Fi"));
    let tb = ui.add(col, ToggleButton::new("Bold"));
    let radios = ui.add(
        col,
        RadioButtons::new("Size", &["Small", "Medium", "Large"]).selected(0),
    );
    ui.edit::<RadioButtons, _>(radios, |r| r.disabled = vec![1]);
    let slider = ui.add(col, Slider::new("Volume", 0, 100, 40).step(5));
    ui.update();
    // Pointer: press and release inside toggles exactly once.
    click(&mut ui, cb);
    assert_eq!(kinds(&mut ui), [Notify::Checked(Some(true))]);
    assert!(node_of(&mut ui, cb).is(States::CHECKED));
    // Keyboard Space walks the three-state cycle (checked, mixed, unchecked).
    ui.focus(tri);
    for expected in [Some(true), None, Some(false)] {
        ui.key(Key::Space, Modifiers::NONE);
        assert_eq!(ui.widget::<CheckBox>(tri).unwrap().state, expected);
    }
    assert_eq!(
        kinds(&mut ui),
        [
            Notify::Checked(Some(true)),
            Notify::Checked(None),
            Notify::Checked(Some(false))
        ]
    );
    ui.focus(sw);
    ui.key(Key::Space, Modifiers::NONE);
    assert!(ui.widget::<ToggleSwitch>(sw).unwrap().on);
    assert_eq!(kinds(&mut ui), [Notify::Toggled(true)]);
    click(&mut ui, tb);
    assert!(ui.widget::<ToggleButton>(tb).unwrap().checked);
    assert_eq!(kinds(&mut ui), [Notify::Toggled(true)]);
    // Radio selection follows the arrows, skips disabled items and wraps.
    ui.focus(radios);
    ui.key(Key::Down, Modifiers::NONE);
    assert_eq!(ui.widget::<RadioButtons>(radios).unwrap().selected, Some(2));
    ui.key(Key::Down, Modifiers::NONE);
    assert_eq!(ui.widget::<RadioButtons>(radios).unwrap().selected, Some(0));
    assert_eq!(
        kinds(&mut ui),
        [
            Notify::SelectionChanged(Some(2)),
            Notify::SelectionChanged(Some(0))
        ]
    );
    let tree = ui.semantics().unwrap().clone();
    let items: std::vec::Vec<_> = tree
        .get(widget_node(radios))
        .unwrap()
        .children
        .iter()
        .map(|c| tree.get(*c).unwrap().clone())
        .collect();
    assert_eq!(items.len(), 3);
    assert!(items.iter().all(|i| i.role == Role::RadioButton));
    assert!(items[0].is(States::SELECTED) && items[1].is(States::DISABLED));
    assert_eq!(items[2].position, Some((3, 3)));
    assert_eq!(
        tree.focus,
        Some(item_node(radios, 0)),
        "focus is on the radio item"
    );
    // Slider: arrows step, Home/End clamp; no notification when the value
    // cannot move.
    ui.focus(slider);
    ui.key(Key::Right, Modifiers::NONE);
    assert_eq!(ui.widget::<Slider>(slider).unwrap().value, 45);
    ui.key(Key::End, Modifiers::NONE);
    ui.take_notifications();
    ui.key(Key::Up, Modifiers::NONE);
    assert!(kinds(&mut ui).is_empty(), "already at the maximum");
    ui.key(Key::Home, Modifiers::NONE);
    assert_eq!(ui.widget::<Slider>(slider).unwrap().value, 0);
    let s = node_of(&mut ui, slider);
    assert_eq!(
        (s.role, s.range.unwrap().max, s.range.unwrap().now),
        (Role::Slider, 100_000, 0)
    );
    // Automation follows the same model.
    ui.accessibility_action(widget_node(slider), Action::SetRange(55_000))
        .unwrap();
    assert_eq!(ui.widget::<Slider>(slider).unwrap().value, 55);
    ui.accessibility_action(item_node(radios, 2), Action::Select)
        .unwrap();
    assert_eq!(ui.widget::<RadioButtons>(radios).unwrap().selected, Some(2));
}

#[test]
fn menus_open_navigate_invoke_and_restore_focus() {
    let mut ui = ui();
    let col = ui.add(ui.root(), Panel::column(8));
    let bar = ui.add(
        col,
        MenuBar::new(vec![
            (
                "File",
                vec![
                    MenuItem::new(1, "New"),
                    MenuItem::separator(),
                    MenuItem::new(2, "Save"),
                ],
            ),
            (
                "Edit",
                vec![
                    MenuItem::new(3, "Cut"),
                    MenuItem::new(4, "Paste").disabled(),
                ],
            ),
        ]),
    );
    let text = ui.add(col, TextBox::new("Document"));
    let flyout = ui.add(
        col,
        MenuFlyout::new(vec![
            MenuItem::new(10, "Copy"),
            MenuItem::new(11, "Properties"),
        ]),
    );
    ui.update();
    ui.focus(text);
    // Alt+F opens File in menu mode; Down skips the separator; Enter invokes,
    // closes and returns focus to where it was.
    ui.key(Key::Char('f'), Modifiers::ALT);
    assert!(ui.widget::<MenuBar>(bar).unwrap().is_open());
    assert_eq!(ui.focused(), Some(bar));
    assert!(
        ui.semantics()
            .unwrap()
            .nodes()
            .any(|n| n.role == Role::MenuItem && n.name == "Save")
    );
    ui.key(Key::Down, Modifiers::NONE);
    ui.key(Key::Enter, Modifiers::NONE);
    assert_eq!(kinds(&mut ui), [Notify::Command(2)]);
    assert!(!ui.widget::<MenuBar>(bar).unwrap().is_open());
    assert_eq!(ui.focused(), Some(text), "menu mode returns focus");
    // F10 enters menu mode; the first Escape closes a menu, the next leaves.
    ui.key(Key::F(10), Modifiers::NONE);
    assert_eq!(ui.focused(), Some(bar));
    assert!(ui.focus_visible());
    ui.key(Key::Down, Modifiers::NONE);
    assert!(ui.widget::<MenuBar>(bar).unwrap().is_open());
    ui.key(Key::Right, Modifiers::NONE);
    assert!(
        ui.semantics()
            .unwrap()
            .nodes()
            .any(|n| n.role == Role::MenuItem && n.name == "Cut"),
        "Right moves to Edit"
    );
    ui.key(Key::Escape, Modifiers::NONE);
    assert!(!ui.widget::<MenuBar>(bar).unwrap().is_open());
    assert_eq!(ui.focused(), Some(bar));
    ui.key(Key::Escape, Modifiers::NONE);
    assert_eq!(ui.focused(), Some(text));
    assert!(
        kinds(&mut ui)
            .iter()
            .all(|k| !matches!(k, Notify::Command(_)))
    );
    // A context flyout takes the keyboard while focus stays on its owner.
    let at = ui.rect(text).unwrap().center();
    ui.command::<MenuFlyout, _>(flyout, |m, cx| m.open_at(cx, at, true));
    assert_eq!(ui.overlays(), [flyout]);
    assert_eq!(ui.focused(), Some(text));
    ui.key(Key::Down, Modifiers::NONE);
    ui.key(Key::Enter, Modifiers::NONE);
    assert_eq!(kinds(&mut ui), [Notify::Command(11)]);
    assert!(ui.overlays().is_empty());
    // Escape and outside clicks dismiss without invoking.
    ui.command::<MenuFlyout, _>(flyout, |m, cx| m.open_at(cx, at, true));
    ui.key(Key::Escape, Modifiers::NONE);
    assert!(ui.overlays().is_empty());
    assert_eq!(kinds(&mut ui), [Notify::Dismissed]);
    ui.command::<MenuFlyout, _>(flyout, |m, cx| m.open_at(cx, at, false));
    ui.pointer_down(LPoint::px(630, 470), MouseButton::Primary, Modifiers::NONE);
    ui.pointer_up(LPoint::px(630, 470), MouseButton::Primary, Modifiers::NONE);
    assert!(ui.overlays().is_empty());
    assert_eq!(kinds(&mut ui), [Notify::Dismissed]);
    assert_eq!(ui.focused(), Some(text));
    // Window deactivation closes popups too.
    ui.command::<MenuFlyout, _>(flyout, |m, cx| m.open_at(cx, at, false));
    ui.window_focus(false);
    assert!(ui.overlays().is_empty());
    ui.window_focus(true);
    assert_eq!(ui.focused(), Some(text));
}

#[test]
fn lists_and_trees_keep_identity_and_hide_collapsed_items() {
    let mut ui = ui();
    let col = ui.add(ui.root(), Panel::column(8));
    let items = |n: u32| {
        (0..n)
            .map(|i| ListItem::new(100 + i, &std::format!("Item {i}")))
            .collect::<std::vec::Vec<_>>()
    };
    let list = ui.add(col, ListView::new("Files", items(5)).select(101));
    let multi = ui.add(
        col,
        ListView::new("Fruits", items(4)).mode(Selection::Multiple),
    );
    let tree = ui.add(
        col,
        TreeView::new(
            "Folders",
            vec![
                TreeItem::new(1, None, "This PC"),
                TreeItem::new(2, Some(1), "Documents"),
                TreeItem::new(3, None, "Network"),
            ],
        ),
    );
    ui.update();
    ui.focus(list);
    ui.key(Key::Down, Modifiers::NONE);
    assert_eq!(ui.widget::<ListView>(list).unwrap().selected, [102]);
    ui.key(Key::End, Modifiers::NONE);
    assert_eq!(ui.widget::<ListView>(list).unwrap().selected, [104]);
    assert_eq!(
        kinds(&mut ui),
        [
            Notify::SelectionChanged(Some(102)),
            Notify::SelectionChanged(Some(104))
        ]
    );
    // Selection survives sorting by identity.
    ui.edit::<ListView, _>(list, |l| l.sort_by(|a, b| b.key.cmp(&a.key)));
    assert_eq!(ui.widget::<ListView>(list).unwrap().selected, [104]);
    let t = ui.semantics().unwrap().clone();
    let l = t.get(widget_node(list)).unwrap();
    assert_eq!(l.role, Role::List);
    let selected: std::vec::Vec<_> = l
        .children
        .iter()
        .map(|c| t.get(*c).unwrap())
        .filter(|n| n.is(States::SELECTED))
        .collect();
    assert_eq!(selected.len(), 1);
    assert_eq!(
        (selected[0].role, selected[0].name.as_str()),
        (Role::ListItem, "Item 4")
    );
    // Multiple selection: Space toggles, Ctrl+A selects everything.
    ui.focus(multi);
    ui.key(Key::Space, Modifiers::NONE);
    ui.key(Key::Char('a'), Modifiers::CTRL);
    assert_eq!(ui.widget::<ListView>(multi).unwrap().selected.len(), 4);
    // Collapsed branches publish no child names; expanding publishes them.
    let names = |ui: &mut Ui| {
        ui.semantics()
            .unwrap()
            .nodes()
            .map(|n| n.name.clone())
            .collect::<std::vec::Vec<_>>()
    };
    assert!(!names(&mut ui).contains(&"Documents".into()));
    ui.focus(tree);
    ui.key(Key::Right, Modifiers::NONE);
    assert!(names(&mut ui).contains(&"Documents".into()));
    let t = ui.semantics().unwrap().clone();
    let pc = t.nodes().find(|n| n.name == "This PC").unwrap();
    assert!(pc.role == Role::TreeItem && pc.is(States::EXPANDED));
    ui.key(Key::Left, Modifiers::NONE);
    assert!(!names(&mut ui).contains(&"Documents".into()));
}

#[test]
fn forced_visual_states_change_paint_only() {
    let mut ui = ui();
    let col = ui.add(ui.root(), Panel::column(8).padding(Insets::all(20)));
    let rest = ui.add(col, Button::new("Same"));
    let shown = ui.add(col, Button::new("Same"));
    ui.update();
    let hash = |ui: &Ui, id: WidgetId| ui.display_list().summary()[&id].0;
    let base_rest = hash(&ui, rest);
    let mut seen = std::vec![hash(&ui, shown)];
    let _ = ui.render(
        &mut Frame::new(&mut vec![0u8; 640 * 480 * 4], 640, 480, 640 * 4).unwrap(),
        door_graphics_core::Scale::ONE,
        &mut Fonts::new(),
        true,
    );
    for v in [VisualState::Hover, VisualState::Pressed, VisualState::Focus] {
        ui.force_visual_state(shown, Some(v));
        ui.update();
        let h = hash(&ui, shown);
        assert!(
            !seen.contains(&h),
            "{v:?} paints differently from the other states"
        );
        seen.push(h);
        assert_eq!(
            ui.damage().map(|d| d.intersects(ui.rect(rest).unwrap())),
            Some(false),
            "only the forced control is damaged"
        );
        let _ = ui.render(
            &mut Frame::new(&mut vec![0u8; 640 * 480 * 4], 640, 480, 640 * 4).unwrap(),
            door_graphics_core::Scale::ONE,
            &mut Fonts::new(),
            false,
        );
    }
    assert!(ui.display_list().ops_of(shown) > 0);
    // Behaviour and semantics keep the real state: a forced-focus button is
    // not focused, a forced-pressed button does not invoke.
    assert!(!node_of(&mut ui, shown).is(States::FOCUSED));
    assert_eq!(ui.focused(), None);
    ui.force_visual_state(shown, Some(VisualState::Pressed));
    ui.update();
    assert!(kinds(&mut ui).is_empty());
    ui.force_visual_state(shown, None);
    assert_eq!(ui.forced_visual_state(shown), None);
    ui.update();
    assert_eq!(hash(&ui, shown), seen[0], "back to the real state");
    assert_eq!(hash(&ui, rest), base_rest);
}

#[test]
fn removal_capture_and_window_activation_never_leave_stale_targets() {
    let mut ui = ui();
    let col = ui.add(ui.root(), Panel::column(8).padding(Insets::all(20)));
    let a = ui.add(col, Button::new("A"));
    let b = ui.add(col, Button::new("B"));
    ui.update();
    // Removing the captured, hovered, focused widget mid-press cancels cleanly.
    press(&mut ui, b);
    ui.remove(b);
    ui.update();
    release_at(&mut ui, LPoint::px(5, 5));
    assert!(kinds(&mut ui).is_empty());
    assert_eq!(ui.focused(), Some(a), "focus moved to the remaining stop");
    // A stale id never resolves after its slot is reused.
    let c = ui.add(col, Button::new("C"));
    assert_eq!(c.index(), b.index());
    assert!(ui.rect(b).is_none() && ui.widget::<Button>(b).is_none());
    assert_eq!(
        ui.accessibility_action(widget_node(b), Action::Invoke),
        Err(ActionError::UnknownNode)
    );
    // Disabling an ancestor moves focus out and blocks input below it.
    ui.update();
    ui.focus(c);
    ui.set_enabled(col, false);
    assert!(ui.focused().is_none());
    click(&mut ui, a);
    assert!(kinds(&mut ui).is_empty());
    assert!(node_of(&mut ui, a).is(States::DISABLED));
    ui.set_enabled(col, true);
    // Deactivation cancels a press and remembers focus.
    ui.focus(a);
    press(&mut ui, a);
    ui.window_focus(false);
    ui.window_focus(true);
    let center = ui.rect(a).unwrap().center();
    release_at(&mut ui, center);
    assert!(kinds(&mut ui).is_empty());
    assert_eq!(ui.focused(), Some(a));
}

/// M7: an open modal dialog owns input for assistive technology too:
/// actions on the window content below it are refused, its own are not.
#[test]
fn assistive_actions_stay_inside_the_active_modal_layer() {
    let mut ui = ui();
    let col = ui.add(ui.root(), Panel::column(8).padding(Insets::all(20)));
    let below = ui.add(col, Button::new("Delete everything"));
    let toggle = ui.add(col, ToggleSwitch::new("Wi-Fi"));
    ui.update();
    let dialog = ContentDialog::build(&mut ui, "Confirm", "", Some("OK"), None, "Cancel");
    ContentDialog::show(&mut ui, &dialog);
    ui.update();
    let _ = ui.take_semantics();
    let _ = kinds(&mut ui);
    assert_eq!(
        ui.accessibility_action(widget_node(below), Action::Invoke),
        Err(ActionError::OutsideModal)
    );
    assert_eq!(
        ui.accessibility_action(widget_node(toggle), Action::Toggle),
        Err(ActionError::OutsideModal)
    );
    assert!(!ui.widget::<ToggleSwitch>(toggle).unwrap().on);
    assert!(kinds(&mut ui).is_empty(), "nothing below the dialog ran");
    // The dialog's own buttons still act.
    let tree = ui.semantics().unwrap().clone();
    let ok = tree
        .nodes()
        .find(|n| n.role == Role::Button && n.name == "OK")
        .unwrap()
        .id;
    assert_eq!(ui.accessibility_action(ok, Action::Invoke), Ok(()));
    // Once it closes the window content acts again.
    ui.update();
    assert_eq!(
        ui.accessibility_action(widget_node(toggle), Action::Toggle),
        Ok(())
    );
}
