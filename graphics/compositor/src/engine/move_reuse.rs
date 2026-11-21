use super::*;

#[derive(Clone, Copy)]
pub(super) struct MoveReuse {
    id: WindowId,
    source: Rect,
    target: Rect,
    epoch: u64,
}

impl Engine {
    fn reusable_window(&self, id: WindowId, bounds: Rect) -> bool {
        self.focused == Some(id)
            && self
                .z_order
                .iter()
                .rev()
                .copied()
                .find(|id| self.visible(*id))
                == Some(id)
            && bounds.intersection(self.work_area) == Some(bounds)
            && self.window(id).is_some_and(|w| {
                w.role == Role::TopLevel
                    && w.state == WindowState::Normal
                    && (!w.caption.is_some_and(|c| c.tabbed)
                        || w.snapshot.as_ref().is_none_or(|s| s.opaque))
            })
    }

    pub(super) fn plan_move_reuse(&self, id: WindowId, old: Rect, new: Rect) -> Option<MoveReuse> {
        if self.caption_damage.is_some()
            || !self.reusable_window(id, old)
            || !self.reusable_window(id, new)
        {
            return None;
        }
        let epoch = self.window(id)?.epoch;
        match self.move_reuse {
            Some(plan) if plan.id == id && plan.target == old && plan.epoch == epoch => {
                Some(MoveReuse {
                    target: new,
                    ..plan
                })
            }
            None if self.damage.is_none() => Some(MoveReuse {
                id,
                source: old,
                target: new,
                epoch,
            }),
            _ => None,
        }
    }

    pub(super) fn take_move_reuse(&mut self, allowed: bool) -> Option<Rect> {
        let plan = self.move_reuse.take()?;
        if !allowed
            || self.caption_damage.is_some()
            || !self.reusable_window(plan.id, plan.source)
            || !self.reusable_window(plan.id, plan.target)
            || !self
                .window(plan.id)
                .is_some_and(|w| w.outer == plan.target && w.epoch == plan.epoch)
        {
            return None;
        }
        // The client interior has full window coverage. Never translate shadow,
        // antialiased corners, trusted title text or title controls.
        let inset = self.scale.len(window_radius());
        let top = self.scale.len(self.window(plan.id)?.title_height());
        let width = plan.target.width().checked_sub(2 * inset)?;
        let height = plan.target.height().checked_sub(top + inset)?;
        let target = Rect::new(
            plan.target.x() + inset as i32,
            plan.target.y() + top as i32,
            width,
            height,
        )
        .ok()?;
        let source = Rect::new(
            plan.source.x() + inset as i32,
            plan.source.y() + top as i32,
            width,
            height,
        )
        .ok()?;
        let stride = self.output.width() as usize * 4;
        let bytes = width as usize * 4;
        // No mutation occurs until all eligibility/geometry checks above pass.
        // copy_within handles horizontal overlap; row order handles vertical overlap.
        for row in 0..height as usize {
            let row = if target.y() > source.y() {
                height as usize - 1 - row
            } else {
                row
            };
            let from = (source.y() as usize + row) * stride + source.x() as usize * 4;
            let to = (target.y() as usize + row) * stride + target.x() as usize * 4;
            self.pixels.copy_within(from..from + bytes, to);
        }
        Some(target)
    }
}

pub(super) fn subtract(bounds: Rect, inner: Rect) -> [Option<Rect>; 4] {
    debug_assert_eq!(bounds.intersection(inner), Some(inner));
    [
        Rect::new(
            bounds.x(),
            bounds.y(),
            bounds.width(),
            (inner.y() - bounds.y()) as u32,
        )
        .ok(),
        Rect::new(
            bounds.x(),
            inner.bottom(),
            bounds.width(),
            (bounds.bottom() - inner.bottom()) as u32,
        )
        .ok(),
        Rect::new(
            bounds.x(),
            inner.y(),
            (inner.x() - bounds.x()) as u32,
            inner.height(),
        )
        .ok(),
        Rect::new(
            inner.right(),
            inner.y(),
            (bounds.right() - inner.right()) as u32,
            inner.height(),
        )
        .ok(),
    ]
}
