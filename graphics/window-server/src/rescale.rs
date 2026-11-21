//! Live display scale (spec §57): SET_DISPLAY_SCALE / DISPLAY_SCALE_INFO and
//! the rescale of the whole desktop. Windows keep their logical geometry;
//! the shell (fonts, taskbar material, overlay underlay, glyphs, cursor)
//! re-renders at the new scale without restarting any application.
use super::*;
use door_window::{display_scale, policy, taskbar};
impl Desktop {
    /// SET_DISPLAY_SCALE: a 4-byte preference (0 = automatic). Unknown or
    /// unoffered percents are refused; the reply is the effective percent.
    pub(super) fn display_scale_request(
        &mut self,
        b: &[u8],
        t: &[door_abi::Transfer],
    ) -> Result<Response, Code> {
        if b.len() != 4 || !t.is_empty() {
            return Err(Code::InvalidRequest);
        }
        let preference = u32::from_le_bytes(b.try_into().map_err(|_| Code::InvalidRequest)?);
        let output = self.engine.output_rect();
        if preference != 0 && !display_scale::offered(preference, output.width(), output.height()) {
            return Err(Code::InvalidRequest);
        }
        let effective = self.apply_display_scale(preference)?;
        Ok(Response::scalar(u64::from(effective)))
    }
    /// DISPLAY_SCALE_INFO reply: preference, effective, recommended, offered.
    pub(super) fn display_scale_info(&self) -> [u8; 16] {
        let output = self.engine.output_rect();
        let recommended =
            policy::recommended_scale(self.scale_hint, output.width(), output.height());
        let mut out = [0; 16];
        for (chunk, value) in out.chunks_exact_mut(4).zip([
            self.scale_preference,
            self.engine.scale().percent(),
            recommended.percent(),
            display_scale::offered_mask(output.width(), output.height()),
        ]) {
            chunk.copy_from_slice(&value.to_le_bytes());
        }
        out
    }
    /// SCALE_CHANGED for a CAP_DISPLAY_SCALE client with one pending.
    pub(super) fn take_scale(&mut self, owner: g::Owner) -> Option<w::Event> {
        let client = self.clients.iter_mut().find(|c| c.owner == owner)?;
        if !core::mem::take(&mut client.scale_pending) {
            return None;
        }
        let output = self.engine.output_rect();
        let scale = self.engine.scale();
        let recommended =
            policy::recommended_scale(self.scale_hint, output.width(), output.height());
        Some(w::Event {
            kind: w::SCALE_CHANGED,
            a: scale.percent(),
            b: scale.dpi(),
            value: u64::from(self.scale_preference) << 32 | u64::from(recommended.percent()),
            rect: w::Rect {
                x: 0,
                y: 0,
                width: output.width(),
                height: output.height(),
            },
            ..w::Event::default()
        })
    }
    /// Resolve `preference` and, when the effective scale changes, rescale
    /// the live desktop. Nothing changes unless every step is admitted.
    pub(super) fn apply_display_scale(&mut self, preference: u32) -> Result<u32, Code> {
        let output = self.engine.output_rect();
        let (width, height) = (output.width(), output.height());
        let scale = policy::resolve_scale(preference, self.scale_hint, width, height);
        if scale == self.engine.scale() {
            self.scale_preference = preference;
            return Ok(scale.percent());
        }
        // Preflight every allocation before the engine publishes anything.
        let fonts = display_scale::shell_fonts_text(scale, self.fonts.text_percent())
            .or_else(|| display_scale::shell_fonts(scale))
            .ok_or(Code::Unsupported)?;
        let logical = (scale.logical_len(width), scale.logical_len(height));
        let layout = taskbar::Layout::with_controls(logical.0, logical.1, self.controls);
        let cache = taskbar::physical(layout.cache(), scale);
        let bytes = cache.width() as usize * cache.height() as usize * 4;
        self.overlay_underlay
            .try_reserve_exact(bytes.saturating_sub(self.overlay_underlay.len()))
            .map_err(|_| Code::Resources)?;
        let material = door_window::material::Taskbar::for_region(
            width,
            height,
            taskbar::physical(layout.bar(), scale),
        )
        .ok_or(Code::Resources)?;
        let old_epochs: Vec<_> = self.engine.windows().map(|w| (w.id, w.epoch)).collect();
        self.engine
            .rescale(scale, taskbar::physical(layout.work_area(), scale))
            .map_err(|e| match e {
                g::Error::Busy | g::Error::QueueFull => Code::WouldBlock,
                g::Error::Quota => Code::Resources,
                _ => Code::InvalidRequest,
            })?;
        // Committed: buffers attached at the old size are stale.
        self.attachments.retain(|a| {
            self.engine.window_info(a.window).is_ok_and(|w| {
                old_epochs
                    .iter()
                    .any(|(id, epoch)| *id == a.window && *epoch == w.epoch)
            })
        });
        self.scale_preference = preference;
        self.fonts = fonts;
        door_window::paint::set_text_percent(self.fonts.text_percent());
        self.overlay_underlay.resize(bytes, 0);
        self.taskbar_material = material;
        // Shell surfaces sized for the old scale close, as on Windows.
        let overlay_owned_focus = self.launcher || self.tray.open.is_some();
        self.close_menu();
        self.launcher = false;
        self.tray.open = None;
        self.overview = None;
        self.assist_dismiss();
        self.drag = None;
        self.caption_press = None;
        self.desktop_press = None;
        self.desktop_selection.cancel_gesture();
        self.icon_hover = None;
        if overlay_owned_focus {
            self.restore_shell_focus();
        }
        self.icons_dirty = true;
        self.bar_dirty = true;
        self.cursor_dirty = true;
        self.last_cursor = None;
        self.presented_overlay = false;
        self.presented_overview = None;
        self.engine.invalidate();
        for client in &mut self.clients {
            client.scale_pending |= client.scale_events;
        }
        // DISPLAY-01: the focused top-level window's outer bounds in logical
        // units (x,y,w,h) after the rescale; they must not change with scale.
        let focus = self
            .engine
            .focused_window()
            .and_then(|id| self.engine.window_info(id).ok())
            .map(|w| {
                let (x, y) = (scale.logical(w.outer.x()), scale.logical(w.outer.y()));
                (
                    x,
                    y,
                    scale.logical(w.outer.right()) - x,
                    scale.logical(w.outer.bottom()) - y,
                )
            });
        let (fx, fy, fw, fh) = focus.unwrap_or((0, 0, 0, 0));
        door_rt::diagnostic(format_args!(
            "DOOR_P12 DISPLAY_SCALE preference={preference} effective={} dpi={} logical={}x{} factor={} focus={fx},{fy},{fw},{fh}\n",
            scale.percent(),
            scale.dpi(),
            logical.0,
            logical.1,
            display_scale::factor_text(scale),
        ));
        Ok(scale.percent())
    }
}
