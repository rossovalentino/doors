//! Trusted placement and workspace policy. These are organization boundaries,
//! not permission domains; process/session ownership never changes here.
use super::*;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WorkspaceId(pub u64);
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Snap {
    Left,
    Right,
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
    Maximize,
}
impl Engine {
    pub fn work_area(&self) -> Rect {
        self.work_area
    }
    pub fn active_workspace(&self) -> WorkspaceId {
        self.workspace
    }
    pub fn workspace_ids(&self) -> impl Iterator<Item = WorkspaceId> + '_ {
        self.workspaces
            .iter()
            .enumerate()
            .filter(|(_, s)| s.1)
            .map(|(i, s)| WorkspaceId(raw(i, s.0)))
    }
    pub fn workspace_exists(&self, id: WorkspaceId) -> bool {
        parts(id.0)
            .ok()
            .and_then(|(i, g)| self.workspaces.get(i).map(|s| s.1 && s.0 == g))
            .unwrap_or(false)
    }
    pub fn create_workspace(&mut self) -> Result<WorkspaceId> {
        let i = self
            .workspaces
            .iter()
            .position(|s| !s.1 && s.0 < u32::MAX)
            .ok_or(Error::Quota)?;
        self.workspaces[i].0 += 1;
        self.workspaces[i].1 = true;
        Ok(WorkspaceId(raw(i, self.workspaces[i].0)))
    }
    pub fn switch_workspace(&mut self, id: WorkspaceId) -> Result<()> {
        if !self.workspace_exists(id) {
            return Err(Error::UnknownId);
        }
        if id == self.workspace {
            return Ok(());
        }
        self.clear_focus();
        if let Some(captured) = self.capture.take() {
            self.interaction_event(captured, Event::CaptureLost { window: captured });
        }
        // Transitions belong to the scene being left.
        self.finish_motion();
        self.workspace = id;
        self.caption_hover = None;
        self.caption_press = None;
        self.caption_left_down = false;
        let next = self.z_order.iter().rev().copied().find(|id| {
            self.window(*id).is_some_and(|w| w.role == Role::TopLevel) && self.visible(*id)
        });
        if let Some(next) = next {
            self.activate(next)?;
        }
        self.invalidate();
        Ok(())
    }
    pub fn move_to_workspace(
        &mut self,
        owner: Owner,
        id: WindowId,
        target: WorkspaceId,
    ) -> Result<()> {
        if !self.workspace_exists(target) {
            return Err(Error::UnknownId);
        }
        if self.owned(owner, id)?.role != Role::TopLevel {
            return Err(Error::InvalidState);
        }
        self.clear_interaction_tree(id);
        let tree: Vec<_> = self
            .z_order
            .iter()
            .copied()
            .filter(|child| self.is_descendant(*child, id))
            .collect();
        for child in tree {
            lookup_mut(&mut self.windows, child.0)?.workspace = target;
        }
        self.invalidate();
        Ok(())
    }
    pub fn remove_workspace(&mut self, id: WorkspaceId) -> Result<WorkspaceId> {
        if !self.workspace_exists(id) {
            return Err(Error::UnknownId);
        }
        let ids: Vec<_> = self.workspace_ids().collect();
        let index = ids.iter().position(|w| *w == id).ok_or(Error::UnknownId)?;
        let target = if index > 0 {
            ids[index - 1]
        } else {
            *ids.get(1).ok_or(Error::InvalidState)?
        };
        // Closing a workspace moves even closing-window records, retaining all
        // captures/processes and any existing close deadline until retirement.
        for w in self
            .windows
            .iter_mut()
            .filter_map(|s| s.value.as_mut())
            .filter(|w| w.workspace == id)
        {
            w.workspace = target;
        }
        self.workspaces[parts(id.0)?.0].1 = false;
        if self.workspace == id {
            self.switch_workspace(target)?;
        }
        self.invalidate();
        Ok(target)
    }
    pub fn snap_window(
        &mut self,
        owner: Owner,
        id: WindowId,
        snap: Snap,
        drag_restore: Option<Rect>,
    ) -> Result<()> {
        self.owned(owner, id)?;
        let area = self.work_area;
        let half = area.width() / 2;
        let height = area.height() / 2;
        let target = match snap {
            Snap::Maximize => area,
            Snap::Left => Rect::new(area.x(), area.y(), half, area.height())?,
            Snap::Right => Rect::new(
                area.x() + half as i32,
                area.y(),
                area.width() - half,
                area.height(),
            )?,
            Snap::TopLeft => Rect::new(area.x(), area.y(), half, height)?,
            Snap::TopRight => Rect::new(
                area.x() + half as i32,
                area.y(),
                area.width() - half,
                height,
            )?,
            Snap::BottomLeft => Rect::new(
                area.x(),
                area.y() + height as i32,
                half,
                area.height() - height,
            )?,
            Snap::BottomRight => Rect::new(
                area.x() + half as i32,
                area.y() + height as i32,
                area.width() - half,
                area.height() - height,
            )?,
        };
        self.place_snapped(owner, id, target, drag_restore, snap == Snap::Maximize)
    }
    /// Snap layout zone: place a top-level window at `zone` (any rectangle
    /// inside the work area, such as a 2/3 column or a stacked quarter) and
    /// mark it snapped exactly like the built-in halves. The pre-snap normal
    /// geometry is kept for restore-on-drag and unsnap: `drag_restore` when a
    /// title drag started from a restored size, otherwise the current normal
    /// geometry (the saved one for an already snapped/maximized window).
    pub fn snap_window_rect(
        &mut self,
        owner: Owner,
        id: WindowId,
        zone: Rect,
        drag_restore: Option<Rect>,
    ) -> Result<()> {
        self.owned(owner, id)?;
        if self.work_area.intersection(zone) != Some(zone) {
            return Err(Error::InvalidGeometry);
        }
        self.place_snapped(owner, id, zone, drag_restore, false)
    }
    fn place_snapped(
        &mut self,
        owner: Owner,
        id: WindowId,
        target: Rect,
        drag_restore: Option<Rect>,
        maximize: bool,
    ) -> Result<()> {
        let w = self.owned(owner, id)?;
        if w.role != Role::TopLevel || w.state == WindowState::Minimized {
            return Err(Error::InvalidState);
        }
        let old = w.outer;
        let visible = self.visible(id);
        let normal = drag_restore.unwrap_or(if w.snapped || w.state == WindowState::Maximized {
            w.restore
        } else {
            w.outer
        });
        self.validate_outer(normal)?;
        self.validate_outer(target)?;
        if target != old {
            self.configure(owner, id, target)?;
        }
        let w = lookup_mut(&mut self.windows, id.0)?;
        w.restore = normal;
        w.snapped = !maximize;
        w.state = if maximize {
            WindowState::Maximized
        } else {
            WindowState::Normal
        };
        self.dirty_tree(id);
        if visible && target != old {
            // Retained pixels glide from the old bounds into the zone.
            self.motion_relocate(id, old);
        }
        Ok(())
    }
    /// Change the output scale of a live scene (Settings > Display > Scale).
    /// Every window keeps its logical (device-independent) geometry: edges
    /// convert old-physical -> logical -> new-physical, then stay reachable
    /// in `work_area` (the new scale's shell reservation). Maximized windows
    /// fill it, snapped windows keep their zone proportions, and saved
    /// restore rectangles convert the same way. Each window receives one
    /// Configure with its new size; nothing is published unless every
    /// geometry and owner queue admits it. Transitions finish first.
    pub fn rescale(&mut self, scale: Scale, work_area: Rect) -> Result<()> {
        if self.in_flight.is_some() {
            return Err(Error::Busy);
        }
        if self.output.intersection(work_area) != Some(work_area)
            || work_area.width() < scale.len(160)
            || work_area.height() < scale.len(64)
        {
            return Err(Error::InvalidGeometry);
        }
        if scale == self.scale {
            return self.reflow_work_area(work_area);
        }
        let old = self.scale;
        let old_area = self.work_area;
        let convert = |r: Rect| -> Option<Rect> {
            let (x0, y0) = (old.logical(r.x()), old.logical(r.y()));
            let (x1, y1) = (old.logical(r.right()), old.logical(r.bottom()));
            scale
                .rect(Rect::new(x0, y0, (x1 - x0).max(1) as u32, (y1 - y0).max(1) as u32).ok()?)
                .ok()
        };
        let reach = |r: Rect| -> Option<Rect> {
            let x = r.x().clamp(
                work_area.x() + scale.len(80) as i32 - r.width() as i32,
                work_area.right() - scale.len(80) as i32,
            );
            let y = r
                .y()
                .clamp(work_area.y(), work_area.bottom() - scale.len(32) as i32);
            Rect::new(x, y, r.width(), r.height()).ok()
        };
        let zone = |r: Rect| -> Option<Rect> {
            let map = |v: i32, from: i32, from_len: u32, to: i32, to_len: u32| {
                to + ((i64::from(v - from) * i64::from(to_len)) / i64::from(from_len)) as i32
            };
            let x = map(
                r.x(),
                old_area.x(),
                old_area.width(),
                work_area.x(),
                work_area.width(),
            );
            let right = map(
                r.right(),
                old_area.x(),
                old_area.width(),
                work_area.x(),
                work_area.width(),
            );
            let y = map(
                r.y(),
                old_area.y(),
                old_area.height(),
                work_area.y(),
                work_area.height(),
            );
            let bottom = map(
                r.bottom(),
                old_area.y(),
                old_area.height(),
                work_area.y(),
                work_area.height(),
            );
            Rect::new(x, y, (right - x) as u32, (bottom - y) as u32).ok()
        };
        let fits = |r: Rect| {
            r.width() >= scale.len(160)
                && r.height() >= scale.len(64)
                && r.width() <= MAX_WIDTH
                && r.height() <= MAX_HEIGHT
        };
        // (id, owner, outer, restore, snapped)
        let mut plans: Vec<(WindowId, Owner, Rect, Rect, bool)> = Vec::new();
        for w in self.windows.iter().filter_map(|s| s.value.as_ref()) {
            if w.state == WindowState::Closing {
                continue;
            }
            let maximized = w.state == WindowState::Maximized
                || w.state == WindowState::Minimized && w.pre_minimize == WindowState::Maximized;
            let top_level = w.role == Role::TopLevel;
            let outer = if top_level && maximized {
                Some(work_area)
            } else if top_level && w.snapped {
                zone(w.outer)
            } else if top_level {
                convert(w.outer).and_then(reach)
            } else {
                convert(w.outer)
            }
            .ok_or(Error::InvalidGeometry)?;
            let restore = if top_level {
                convert(w.restore).and_then(reach)
            } else {
                convert(w.restore)
            }
            .ok_or(Error::InvalidGeometry)?;
            if !fits(outer) || !fits(restore) {
                return Err(Error::InvalidGeometry);
            }
            w.epoch.checked_add(1).ok_or(Error::Quota)?;
            plans.push((w.id, w.owner, outer, restore, w.snapped));
        }
        for (_, owner, ..) in &plans {
            let queued_configures = self
                .client_index(*owner)
                .map(|i| {
                    self.clients[i]
                        .events
                        .iter()
                        .filter(|e| matches!(e, Event::Configure { .. }))
                        .count()
                })
                .unwrap_or(0);
            let needed = plans.iter().filter(|p| p.1 == *owner).count();
            // Superseded configures are coalesced by `configure`.
            if self
                .room(*owner, needed.saturating_sub(queued_configures))
                .is_err()
            {
                return Err(Error::QueueFull);
            }
        }
        self.finish_motion();
        self.scale = scale;
        self.work_area = work_area;
        self.caption_hover = None;
        self.caption_press = None;
        for (id, owner, outer, restore, snapped) in plans {
            lookup_mut(&mut self.windows, id.0)?.scale = scale;
            self.configure(owner, id, outer)?;
            let w = lookup_mut(&mut self.windows, id.0)?;
            w.snapped = snapped;
            w.restore = restore;
        }
        // Closing windows keep their geometry until destroyed.
        for slot in &mut self.windows {
            if let Some(w) = slot.value.as_mut() {
                w.scale = scale;
            }
        }
        self.invalidate();
        Ok(())
    }
    pub fn unsnap_window(&mut self, owner: Owner, id: WindowId) -> Result<()> {
        let w = self.owned(owner, id)?;
        if !w.snapped {
            return self.restore_window(owner, id);
        }
        let target = w.restore;
        self.configure(owner, id, target)?;
        Ok(())
    }

    /// Restore a maximized/snapped title drag atomically under its pointer.
    /// Size comes from the saved normal geometry; horizontal anchoring preserves
    /// the press fraction across the title. A failed configure changes no bounds.
    pub fn restore_title_drag(
        &mut self,
        owner: Owner,
        id: WindowId,
        pressed: (i32, i32),
        pointer: (i32, i32),
    ) -> Result<Rect> {
        let w = self.owned(owner, id)?;
        if w.role != Role::TopLevel
            || !matches!(w.state, WindowState::Normal | WindowState::Maximized)
            || !(w.snapped || w.state == WindowState::Maximized)
        {
            return Err(Error::InvalidState);
        }
        let old = w.outer;
        if !old.contains(pressed.0, pressed.1)
            || pressed.1 >= old.y() + self.scale.len(w.title_height()) as i32
        {
            return Err(Error::InvalidGeometry);
        }
        let normal = w.restore;
        let anchor = (i64::from(pressed.0 - old.x()) * i64::from(normal.width())
            / i64::from(old.width())) as i32;
        let scale = self.scale;
        let x = pointer.0.saturating_sub(anchor).clamp(
            self.work_area.x() + scale.len(80) as i32 - normal.width() as i32,
            self.work_area.right() - scale.len(80) as i32,
        );
        let y = pointer.1.saturating_sub(pressed.1 - old.y()).clamp(
            self.work_area.y(),
            self.work_area.bottom() - scale.len(TITLE_HEIGHT) as i32,
        );
        let target = Rect::new(x, y, normal.width(), normal.height())?;
        self.configure(owner, id, target)?;
        let w = lookup_mut(&mut self.windows, id.0)?;
        w.state = WindowState::Normal;
        w.restore = target;
        self.dirty_tree(id);
        Ok(target)
    }
}
