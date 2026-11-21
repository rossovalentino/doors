//! ContentDialog: a modal layer with a title, body, optional content and up
//! to three buttons. Escape closes with `RESULT_NONE`; Enter (when the
//! focused control leaves it unhandled) chooses the primary button. Focus is
//! trapped while open and restored when it closes (`Ui::open_layer`).
use super::basic::{Button, Panel, TextBlock};
use super::describe;
use crate::WidgetId;
use crate::event::{Event, Key, Phase};
use crate::layout::{Align, Axis, Style, Track};
use crate::theme::TextRole;
use crate::ui::{LayerOptions, Ui};
use crate::units::{Insets, LRect, Lu};
use crate::widget::{EventCx, Notify, PaintCx, SemanticsCx, Widget};
use alloc::{string::String, vec::Vec};
use door_accessibility::{Actions, Node, Role};

pub const RESULT_NONE: u32 = 0;
pub const RESULT_PRIMARY: u32 = 1;
pub const RESULT_SECONDARY: u32 = 2;
/// Routed commands of the footer buttons (base + result).
pub const DIALOG_COMMAND: u32 = 0xfffd_0000;

pub struct ContentDialog {
    pub title: String,
    has_primary: bool,
}
/// The parts `ContentDialog::build` creates.
#[derive(Clone, Copy, Debug)]
pub struct Dialog {
    pub root: WidgetId,
    /// Add custom content here (below the body text).
    pub content: WidgetId,
    pub footer: WidgetId,
}
impl ContentDialog {
    /// Build a dialog (not yet shown): `primary` is the accent default
    /// button; `close` is always present.
    pub fn build(
        ui: &mut Ui,
        title: &str,
        body: &str,
        primary: Option<&str>,
        secondary: Option<&str>,
        close: &str,
    ) -> Dialog {
        let root = ui.create(ContentDialog {
            title: title.into(),
            has_primary: primary.is_some(),
        });
        let content = ui.add(root, Panel::column(12).padding(Insets::all(24)));
        ui.add(
            content,
            TextBlock::new(title).role(TextRole::Subtitle).heading(1),
        );
        if !body.is_empty() {
            ui.add(content, TextBlock::new(body).wrapping());
        }
        let buttons: Vec<(String, u32, bool)> = [
            primary.map(|p| (String::from(p), RESULT_PRIMARY, true)),
            secondary.map(|s| (String::from(s), RESULT_SECONDARY, false)),
            Some((String::from(close), RESULT_NONE, false)),
        ]
        .into_iter()
        .flatten()
        .collect();
        let columns = buttons.iter().map(|_| Track::Star(1)).collect();
        let footer = ui.add(
            root,
            Panel::grid(columns, alloc::vec![Track::Auto], 8).padding(Insets::all(24)),
        );
        for (i, (label, result, accent)) in buttons.into_iter().enumerate() {
            let mut b = Button::new(&label).command(DIALOG_COMMAND + result);
            if accent {
                b = b.accent();
            }
            let id = ui.add(footer, b);
            ui.style_mut(id, |s| {
                *s = s
                    .clone()
                    .cell(0, i as u16)
                    .align(Align::Stretch, Align::Stretch);
            });
        }
        Dialog {
            root,
            content,
            footer,
        }
    }
    /// Show a built dialog modally.
    pub fn show(ui: &mut Ui, dialog: &Dialog) {
        ui.open_layer(dialog.root, LayerOptions::modal());
    }
    fn finish(&self, cx: &mut EventCx<'_>, result: u32) {
        cx.notify(Notify::Closed(result));
        cx.close_layer();
        cx.stop();
    }
}
impl Widget for ContentDialog {
    fn type_name(&self) -> &'static str {
        "ContentDialog"
    }
    fn style(&self) -> Style {
        Style::stack(Axis::Vertical, 0)
            .min_size(320, 0)
            .max_size(548, 100_000)
    }
    fn paint(&self, cx: &mut PaintCx<'_>) {
        let c = cx.theme.colors;
        let r = cx.rect;
        cx.shadow(r, 8, 8);
        cx.round(r, 8, c.flyout_stroke);
        cx.round(r.inflate(-Lu::ONE), 7, c.card);
        let footer = Lu::px(cx.theme.metrics.control_height)
            .max(cx.text.line_height(TextRole::Body) + Lu::px(12))
            + Lu::px(48);
        let f = LRect::new(
            r.x + Lu::ONE,
            r.bottom() - footer,
            r.w - Lu::px(2),
            footer - Lu::ONE,
        );
        cx.fill(LRect::new(f.x, f.y, f.w, f.h - Lu::px(8)), c.background);
        cx.round(LRect::new(f.x, f.y, f.w, f.h), 7, c.background);
        cx.fill(LRect::new(f.x, f.y, f.w, Lu::ONE), c.card_stroke);
    }
    fn event(&mut self, cx: &mut EventCx<'_>, e: &Event) {
        if cx.phase == Phase::Capture || cx.is_handled() {
            return;
        }
        match e {
            Event::Command(cmd)
                if (DIALOG_COMMAND..=DIALOG_COMMAND + RESULT_SECONDARY).contains(cmd) =>
            {
                self.finish(cx, cmd - DIALOG_COMMAND);
            }
            Event::KeyDown(k) if k.key == Key::Escape => self.finish(cx, RESULT_NONE),
            Event::KeyDown(k) if k.key == Key::Enter && k.modifiers.none() && self.has_primary => {
                self.finish(cx, RESULT_PRIMARY)
            }
            _ => (),
        }
    }
    fn semantics(&self, _cx: &SemanticsCx<'_>, node: &mut Node) -> bool {
        describe(node, Role::Dialog, &self.title);
        node.actions |= Actions::DISMISS;
        true
    }
    fn action(
        &mut self,
        cx: &mut EventCx<'_>,
        _item: Option<u32>,
        a: &door_accessibility::Action,
    ) -> bool {
        if *a == door_accessibility::Action::Dismiss {
            self.finish(cx, RESULT_NONE);
            return true;
        }
        false
    }
}
