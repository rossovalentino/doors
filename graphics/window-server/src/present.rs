//! CPU-copy backend. Feedback means completion of checked copies into GOP memory;
//! this backend cannot observe vblank or claim tear-free presentation.
use super::*;
use door_graphics_text::{Clip, Target, TextRun, TextStyle};
use door_window::telemetry::{Cutoff, FrameKind, FrameWork, TotalsLine};
/// One checked GOP copy under the DISPLAY-01 presentation policy: the
/// test-image hook may fail it on purpose, and any failure is recorded so the
/// frame loop falls back (full repaint after a bounded backoff) instead of
/// ending the session.
fn scanout(
    display: Handle,
    presentation: &mut door_window::scanout::Presentation,
    offset: u64,
    bytes: &[u8],
) -> Result<(), Status> {
    if presentation.inject() {
        presentation.copy_failed();
        return Err(Status::IoFailure);
    }
    // Night light and colour filters apply to every copied span.
    let result = door_sys::display_write(display, offset, presentation.transformed(bytes));
    if result.is_err() {
        presentation.copy_failed();
    }
    result
}
fn restore_cache(scene: &mut [u8], stride: usize, cache: &[u8], bounds: g::Rect, region: g::Rect) {
    if let Some(r) = bounds.intersection(region) {
        for y in r.y() as usize..r.bottom() as usize {
            let out = y * stride + r.x() as usize * 4;
            let src = ((y - bounds.y() as usize) * bounds.width() as usize + r.x() as usize
                - bounds.x() as usize)
                * 4;
            let len = r.width() as usize * 4;
            scene[out..out + len].copy_from_slice(&cache[src..src + len]);
        }
    }
}
fn save_cache(cache: &mut [u8], bounds: g::Rect, scene: &[u8], stride: usize, region: g::Rect) {
    if let Some(r) = bounds.intersection(region) {
        for y in r.y() as usize..r.bottom() as usize {
            let src = y * stride + r.x() as usize * 4;
            let dest = ((y - bounds.y() as usize) * bounds.width() as usize + r.x() as usize
                - bounds.x() as usize)
                * 4;
            let len = r.width() as usize * 4;
            cache[dest..dest + len].copy_from_slice(&scene[src..src + len]);
        }
    }
}
impl Desktop {
    /// DISPLAY-01: account for one presentation attempt of `kind`. A failed
    /// checked copy is not fatal: every surface is marked for a full repaint
    /// (the output may hold a partial frame), the next attempt waits out a
    /// bounded backoff while input and requests are still serviced, and
    /// Ok(false) is returned. Other errors are internal and remain fatal.
    pub(super) fn presented(
        &mut self,
        kind: &str,
        result: Result<(), Status>,
    ) -> Result<bool, Status> {
        let copy_failed = self.presentation.take_copy_failure();
        match result {
            Ok(()) => {
                if kind == "scene"
                    && let Some(failures) = self.presentation.recovered()
                {
                    door_rt::diagnostic(format_args!(
                        "DOOR_P12 PRESENT_RECOVERED failures={failures} full={} total={}\n",
                        u8::from(self.last_present_full),
                        self.presentation.total_failures()
                    ));
                }
                Ok(true)
            }
            Err(error) if copy_failed => {
                let (failures, delay) = self.presentation.attempt_failed(door_sys::time()?);
                self.engine.invalidate();
                self.bar_dirty = true;
                self.icons_dirty = true;
                self.cursor_dirty = true;
                if failures <= 8 || failures.is_power_of_two() {
                    door_rt::diagnostic(format_args!(
                        "DOOR_P12 PRESENT_FAILED kind={kind} status={error:?} consecutive={failures} retry_ticks={delay}\n"
                    ));
                }
                Ok(false)
            }
            Err(error) => Err(error),
        }
    }
    pub(super) fn present(&mut self) -> Result<(), Status> {
        self.profile(FrameKind::Scene, Self::present_scene)
    }
    pub(super) fn present_captions(&mut self) -> Result<(), Status> {
        self.profile(FrameKind::Caption, Self::paint_captions)
    }
    pub(super) fn present_icons(&mut self) -> Result<(), Status> {
        self.profile(FrameKind::Icons, Self::paint_icons)
    }
    pub(super) fn present_bar(&mut self) -> Result<(), Status> {
        self.profile(FrameKind::Bar, Self::paint_bar)
    }
    fn profile(
        &mut self,
        kind: FrameKind,
        paint: fn(&mut Self, &mut FrameWork) -> Result<(), Status>,
    ) -> Result<(), Status> {
        let started_cycles = door_window::telemetry::cycles();
        let mut work = FrameWork::new(kind, self.drag.is_some(), door_sys::time()?);
        let result = paint(self, &mut work);
        // Successful paths already sampled checked-transfer completion. Failure
        // paths retain attempted calls/accepted bytes without losing the sample.
        let at = work
            .completed
            .or_else(|| door_sys::time().ok())
            .unwrap_or(work.started);
        let ended_cycles = door_window::telemetry::cycles();
        if result.is_ok() {
            self.frame_clock.frame(
                started_cycles,
                work.composed_cycles.unwrap_or(ended_cycles),
                ended_cycles,
                at,
            );
            self.trace_animation();
        }
        self.motion.record(work.finish(at, result.is_err()));
        result
    }
    /// MOTION-01: feed this presented frame to the animation trace. Engine
    /// transitions and shell surfaces are judged at the clock value the
    /// frame was composed for (the engine's last `advance`).
    fn trace_animation(&mut self) {
        use door_window::telemetry::{ANIMATION_FLYOUT, ANIMATION_SNAP_UI, ANIMATION_START};
        let engine = self.engine.transition_activity();
        let composed_us = engine.now.saturating_mul(1000);
        let mut kinds = engine.kinds;
        let mut start = if kinds == 0 { u64::MAX } else { engine.start };
        for (kind, motion) in [
            (ANIMATION_START, Some(self.shell.start_motion)),
            (ANIMATION_FLYOUT, Some(self.shell.flyout_motion)),
            (ANIMATION_SNAP_UI, self.shell.snap.map(|s| s.motion)),
            (
                ANIMATION_SNAP_UI,
                self.shell.assist.as_ref().map(|a| a.motion),
            ),
        ] {
            if let Some(since) = motion.and_then(|m| m.animating_since(composed_us)) {
                kinds |= kind;
                start = start.min(since / 1000);
            }
        }
        let presented = self.shell_now() / 1000;
        if let Some(record) = self.presentation_log.animation.frame(
            engine.now,
            kinds,
            if kinds == 0 { 0 } else { start },
            presented,
        ) {
            door_rt::diagnostic(format_args!("{record}"));
        }
    }
    fn finish_transfer(
        &mut self,
        serial: u64,
        success: bool,
        work: &mut FrameWork,
    ) -> Result<(), Status> {
        let at = door_sys::time()?;
        work.completed = Some(at);
        self.engine
            .complete_transfer(serial, success, at)
            .map_err(|_| Status::IoFailure)
    }
    pub(super) fn report_motion(&mut self, cutoff: Cutoff) {
        if !core::mem::replace(&mut self.frame_time_reported, true)
            && let Ok(tick) = door_sys::time()
            && let Some(line) = self.frame_clock.summary(
                door_window::telemetry::cycles(),
                tick,
                door_abi::platform::MONOTONIC_TICK_MILLIS,
            )
        {
            door_rt::diagnostic(format_args!("{line}"));
        }
        if let Some(groups) = self.motion.take_summary() {
            for (index, totals) in groups.into_iter().enumerate() {
                door_rt::diagnostic(format_args!(
                    "{}",
                    TotalsLine {
                        dragging: index == 1,
                        totals,
                        cutoff,
                    }
                ));
            }
        }
    }
    fn present_scene(&mut self, work: &mut FrameWork) -> Result<(), Status> {
        let started = work.started;
        let binding_presentation = self.binding_presentation.take();
        let material_refreshed = core::cell::Cell::new(false);
        if let Some(overview) = self.overview.as_mut() {
            overview.refresh(&self.engine);
            self.engine.invalidate();
        }
        let overview = &self.overview;
        // Input is serviced during scanout. Record the mode being rendered,
        // rather than the possibly newer gesture state after the copy.
        let overview_mode = overview.as_ref().map(|v| v.is_switcher());
        let workspaces: Vec<_> = self.engine.workspace_ids().collect();
        let active_workspace = self.engine.active_workspace();
        if self.session.is_some() {
            self.sync_shell();
        }
        let shell_now = self.shell_now();
        let buttons = self.taskbar_buttons();
        let extras = self.taskbar_extras();
        let marks = self.taskbar_marks();
        let task_view = self.overview.as_ref().is_some_and(|v| !v.is_switcher());
        let icons = self.desktop_icons();
        let locked = self.shell.locked;
        let overlay_open = self.shell_overlay_visible();
        let overview = &self.overview;
        let fonts = &self.fonts;
        let signed_in = self.session.is_some();
        let partial = !signed_in && self.login_reported
            || signed_in
                && overview.is_none()
                && !overlay_open
                && !self.presented_overlay
                && !self.icons_dirty;
        if !partial {
            self.engine.invalidate();
        }
        let gate = self.gate_view();
        let scale = self.engine.scale();
        let pointer = (scale.logical(self.x), scale.logical(self.y));
        let taskbar_word = self.taskbar_layout().taskbar();
        let dark = self.dark;
        let background = self.background;
        // "Show accent colour on Start and taskbar": the material takes the
        // accent's dark shade instead of the neutral tint.
        let start_tint = self
            .shell
            .prefs
            .accent_on_start(dark, self.controls.contrast().is_some())
            .then(|| {
                let t = door_window::preferences::start_tint(self.resolve_accent());
                [t[0], t[1], t[2]]
            });
        let classes = &self.app_classes;
        let presented_app = self
            .engine
            .focused_window()
            .and_then(|id| self.engine.window_info(id).ok())
            .and_then(|info| classes.iter().find(|(owner, _)| *owner == info.owner))
            .map_or(0, |(_, app)| app.command());
        let presented_scene =
            self.presentation_log
                .capture(presented_app, overlay_open, overview_mode);
        let wallpaper = &self.wallpaper;
        let lock_pixels = &self.lock_pixels;
        let icons_dirty = self.icons_dirty;
        // Not-responding Win32 windows: frozen frame, whitened, title suffix.
        let ghosts: Vec<g::WindowId> = self
            .engine
            .windows()
            .filter(|w| self.shell.hang.window_hung(w.owner, w.id.0))
            .map(|w| w.id)
            .collect();
        let snip_windows = if self.shell.snip.is_some() {
            self.snip_windows()
        } else {
            Vec::new()
        };
        let snip_ticks = door_sys::time().unwrap_or(0);
        let shell = core::cell::RefCell::new(&mut self.shell);
        let tray = core::cell::RefCell::new(&mut self.tray);
        let bar = core::cell::RefCell::new(&mut self.overlay_underlay);
        let material = &mut self.taskbar_material;
        let output_rect = self.engine.output_rect();
        let controls = self.controls;
        let layout = door_window::taskbar::Layout::with_controls(
            scale.logical_len(output_rect.width()),
            scale.logical_len(output_rect.height()),
            controls,
        );
        // Caches and the material cover the shown bar; painting and hits
        // follow the behaviours word (labels, Task view, auto-hide).
        let cached = door_window::taskbar::physical(layout.cache(), scale);
        let bar_region = door_window::taskbar::physical(layout.shown_bar(), scale);
        let layout = layout.with_taskbar(taskbar_word);
        let bar_restored = core::cell::Cell::new(false);
        // Split damage regions only when the overlay is the taskbar alone,
        // whose underlay is restored above (pixels between regions are kept
        // from the previous frame, exactly like move reuse).
        self.engine.allow_split_regions(partial && signed_in);
        let frame = self
            .engine
            .render_damage_reusing_moves(
                partial && signed_in && work.dragging() && !self.bar_dirty && ghosts.is_empty(),
                |canvas| {
                    if partial && signed_in && !bar_restored.replace(true) {
                        restore_cache(
                            canvas.pixels_mut(),
                            output_rect.width() as usize * 4,
                            &bar.borrow(),
                            cached,
                            bar_region,
                        );
                    }
                    let _ = canvas.set_scale(scale);
                    if signed_in {
                        desktop::background(canvas, fonts, wallpaper, icons.as_ref());
                    } else {
                        let _ = canvas.restore_bgra(wallpaper);
                    }
                    let _ = canvas.set_scale(g::Scale::ONE);
                },
                |info, canvas| {
                    let ghost = ghosts.contains(&info.id);
                    if ghost {
                        // The frozen last frame under a whitened veil, as
                        // Windows draws a not-responding window.
                        let _ = canvas.set_scale(g::Scale::ONE);
                        let _ = canvas.fill_rect(info.content, [128, 128, 128, 128]);
                    }
                    if info.client_caption {
                        return;
                    }
                    let ghost_title;
                    let title = if ghost {
                        ghost_title = door_window::not_responding::ghost_title(info.title);
                        ghost_title.as_str()
                    } else {
                        info.title
                    };
                    if let Some(r) = g::Engine::title_clip_scaled(info.outer, scale)
                        .and_then(|r| canvas.clip_rect().and_then(|clip| r.intersection(clip)))
                    {
                        let _ = canvas.set_scale(scale);
                        desktop::caption(
                            canvas,
                            info,
                            classes
                                .iter()
                                .find(|(owner, _)| *owner == info.owner)
                                .map(|(_, app)| *app),
                        );
                        let (w, h, stride) = canvas.physical_dimensions();
                        // Server-drawn (Win32) title bars keep ClearType.
                        if let Ok(target) = Target::new(canvas.pixels_mut(), w, h, stride) {
                            let mut target = target.cleartype();
                            let _ = fonts.draw_physical(
                                &mut target,
                                Clip::new(r.x(), r.y(), r.width(), r.height()),
                                TextRun {
                                    text: title,
                                    x: scale.from_edge(info.outer.x(), 28),
                                    top: scale.from_edge(info.outer.y(), 9),
                                    style: TextStyle::Regular12,
                                    color: if dark {
                                        [240, 240, 240, 255]
                                    } else if info.active {
                                        [32, 32, 32, 255]
                                    } else {
                                        [100, 100, 100, 255]
                                    },
                                },
                            );
                        }
                        let _ = canvas.set_scale(g::Scale::ONE);
                    }
                },
                |canvas| {
                    let clip = canvas.clip_rect().unwrap_or(output_rect);
                    let blur_region = g::Rect::new(
                        (bar_region.x() - 12).max(0),
                        (bar_region.y() - 12).max(0),
                        (bar_region.right() + 12).min(output_rect.right()) as u32
                            - (bar_region.x() - 12).max(0) as u32,
                        (bar_region.bottom() + 12).min(output_rect.bottom()) as u32
                            - (bar_region.y() - 12).max(0) as u32,
                    )
                    .unwrap();
                    // The cache holds the whole scene beneath shell surfaces.
                    // With only the taskbar on screen, partial frames refresh
                    // just its band; opening any surface forces a full frame,
                    // so the rest of the cache is current whenever it is read.
                    let saved = if overlay_open {
                        Some(clip)
                    } else {
                        clip.intersection(blur_region)
                    };
                    if let Some(saved) = saved {
                        save_cache(
                            &mut bar.borrow_mut(),
                            cached,
                            canvas.pixels_mut(),
                            output_rect.width() as usize * 4,
                            saved,
                        );
                    }
                    // Signed-out composition returns before reading this cache.
                    // On admission, full damage intersects the taskbar and
                    // initializes its actual signed-in backdrop before drawing.
                    if signed_in && (!partial || clip.intersection(blur_region).is_some()) {
                        assert!(material.update_tinted(canvas.pixels_mut(), dark, start_tint));
                        material_refreshed.set(true);
                    }
                    if signed_in {
                        canvas.set_clip(output_rect);
                    }
                    let _ = canvas.set_scale(scale);
                    if !signed_in {
                        door_window::login::draw_gate(canvas, fonts, &gate);
                        let _ = canvas.set_scale(g::Scale::ONE);
                        return;
                    }
                    if let Some(overview) = overview {
                        // Task View contains previews, not full-size app windows
                        // showing through a tint. Preserve the real wallpaper.
                        canvas.pixels_mut().copy_from_slice(wallpaper);
                        overview.draw(
                            canvas,
                            fonts,
                            &workspaces,
                            active_workspace,
                            dark,
                            background,
                        );
                    }
                    if locked {
                        // Lock screen: its picture (or the wallpaper) and
                        // the sign-in card only.
                        if lock_pixels.len() == wallpaper.len() {
                            canvas.pixels_mut().copy_from_slice(lock_pixels);
                        } else {
                            canvas.pixels_mut().copy_from_slice(wallpaper);
                        }
                        door_window::login::draw_gate(canvas, fonts, &gate);
                        let _ = canvas.set_scale(g::Scale::ONE);
                        return;
                    }
                    let underlay = bar.borrow();
                    let snip_pointer = pointer;
                    desktop::overlay(
                        canvas,
                        desktop::Overlay {
                            fonts,
                            layout,
                            buttons: &buttons,
                            extras: &extras,
                            marks: &marks,
                            shell: &mut shell.borrow_mut(),
                            tray: &mut tray.borrow_mut(),
                            pointer,
                            dark,
                            material: material.pixels(),
                            underlay: &underlay,
                            now: shell_now,
                            task_view,
                            rebuild: true,
                        },
                    );
                    let _ = canvas.set_scale(g::Scale::ONE);
                    // The snip overlay covers everything: the frozen frame
                    // (taken here once the delay is over), the dim and its bar.
                    if let Some(snip) = shell.borrow_mut().snip.as_mut() {
                        if !snip.frozen() && snip_ticks >= snip.freeze_at {
                            let (pw, ph, _) = canvas.physical_dimensions();
                            snip.freeze(canvas.pixels_mut(), pw, ph, snip_windows.clone());
                        }
                        let _ = canvas.set_scale(scale);
                        snip.draw(canvas, dark, snip_pointer);
                        let _ = canvas.set_scale(g::Scale::ONE);
                    }
                },
            )
            .map_err(|_| Status::IoFailure)?;
        self.shell.presented =
            desktop::regions(&self.shell, &self.tray, layout, shell_now, &self.fonts);
        let serial = frame.serial;
        let reused_pixels = frame.reused_pixels;
        let composed = door_sys::time()?;
        work.mark_composed(composed);
        let width = frame.width;
        let height = frame.height;
        let damage = frame
            .damage
            .unwrap_or(g::Rect::new(0, 0, width, height).unwrap());
        // DISPLAY-01: a recovery is reported only for a whole-output frame.
        self.last_present_full = damage == g::Rect::new(0, 0, width, height).unwrap();
        // Separately composed regions (engine split damage); transfer each
        // instead of their bounding box.
        let mut regions = frame.regions;
        if regions.iter().all(Option::is_none) {
            regions[0] = Some(damage);
        }
        // Magnifier: the whole output goes out magnified from this frame.
        let magnifying = if let Some(m) = self.shell.magnifier {
            let output = self
                .engine
                .transfer_view(serial)
                .map_err(|_| Status::IoFailure)?;
            m.render(
                output.pixels,
                output.stride_bytes,
                width,
                height,
                (self.x, self.y),
                &mut self.shell.magnified,
            );
            let full = self.shell.magnified.len() == width as usize * height as usize * 4;
            if full {
                for r in regions.iter_mut() {
                    *r = None;
                }
                regions[0] = Some(g::Rect::new(0, 0, width, height).unwrap());
            }
            full
        } else {
            false
        };
        // SCENE_PRESENTED reports whether shell surfaces remain on screen
        // after this frame (as `overlay_scene_presented` does). `overlay_open`
        // also counts last frame's surfaces, so the frame that removes a
        // surface without a dismissal animation (reduced motion) would
        // otherwise be reported with overlay=1 and no later frame corrects it.
        // Taken before the transfer services input, like the rest of the scene.
        let presented_scene = door_window::telemetry::Scene {
            overlay: self.shell_overlay_visible(),
            ..presented_scene
        };
        let covered = |r: g::Rect| {
            regions
                .iter()
                .flatten()
                .any(|c| c.intersection(r) == Some(r))
        };
        // Retain the previous overlay's extent for its dismissal frame. An app
        // commit with no flyout otherwise needs only the actual taskbar band.
        let bar = door_window::taskbar::physical(
            layout.overlay(overlay_open || self.presented_overlay),
            scale,
        );
        let origin = layout.work_area();
        let icons = icons_dirty
            .then(|| door_window::taskbar::physical(desktop::icon_column(origin), scale));
        let mut transferred = Ok(());
        self.bar_dirty = false;
        self.icons_dirty = false;
        let mut copied = 0usize;
        let mut total_bytes = 0usize;
        let mut calls = 0usize;
        // Composition remains private; only damaged rows and
        // the trusted overlay reach GOP. Keep the old cursor location so the
        // cursor plane restores it even when it lies outside the damage.
        'regions: for region in regions
            .into_iter()
            .chain([
                (signed_in && !covered(bar)).then_some(bar),
                icons.filter(|region| !covered(*region)),
            ])
            .flatten()
            // Only the output itself is ever transferred.
            .filter_map(|r| r.intersection(g::Rect::new(0, 0, width, height).unwrap()))
        {
            for span in
                door_window::scanout::Spans::new(width, height, region).ok_or(Status::IoFailure)?
            {
                let output = self
                    .engine
                    .transfer_view(serial)
                    .map_err(|_| Status::IoFailure)?;
                work.calls = work.calls.saturating_add(1);
                let source = if magnifying && self.shell.magnified.len() >= span.end {
                    &self.shell.magnified[span.clone()]
                } else {
                    &output.pixels[span.clone()]
                };
                if let Err(error) = scanout(
                    self.display,
                    &mut self.presentation,
                    (span.start / 4) as u64,
                    source,
                ) {
                    transferred = Err(error);
                    break 'regions;
                }
                copied += span.len();
                work.bytes = work.bytes.saturating_add(span.len() as u64);
                total_bytes += span.len();
                calls += 1;
                if copied >= 32 * 1024 {
                    copied = 0;
                    self.input()?;
                    if self.cursor_dirty {
                        self.present_cursor_on(Some(serial))?;
                    }
                }
            }
        }
        self.finish_transfer(serial, transferred.is_ok(), work)?;
        transferred?;
        let completed = work.completed.unwrap_or(composed);
        if reused_pixels > 0 && !self.move_reuse_reported {
            door_rt::diagnostic(format_args!(
                "DOOR_P12 MOVE_REUSE serial={serial} pixels={reused_pixels} bytes={total_bytes} calls={calls}\n"
            ));
            self.move_reuse_reported = true;
        }
        if let Some((bytes, login_presented)) = binding_presentation {
            door_rt::diagnostic(format_args!(
                "DOOR_P12 SESSION_PRESENTED bind_bytes={bytes} login_presented={} full={} material={} frame={serial}\n",
                u8::from(login_presented),
                u8::from(damage == output_rect),
                u8::from(material_refreshed.get()),
            ));
        }
        if self
            .presentation_log
            .frame_sample(serial, completed.saturating_sub(started))
        {
            door_rt::diagnostic(format_args!(
                "DOOR_P12 FRAME serial={serial} compose_ticks={} transfer_ticks={} bytes={total_bytes} calls={calls} drag={}\n",
                composed.saturating_sub(started),
                completed.saturating_sub(composed),
                u8::from(work.dragging())
            ));
        }
        self.presented_overlay = overlay_open;
        self.present_cursor()?;
        if self.presentation_log.superseded(presented_scene) {
            // An activation serviced during scanout needs a subsequent completed
            // scene even when it raised an already-focused, unchanged window.
            self.engine.invalidate();
        } else if self.presentation_log.presented(presented_scene) {
            let _ = door_sys::write(
                format!(
                    // No frame serial: 16 KiB of per-process diagnostics
                    // must last a long session; nothing consumes it here.
                    "DOOR_P12 SCENE_PRESENTED app={presented_app} overlay={}\n",
                    u8::from(presented_scene.overlay)
                )
                .as_bytes(),
            );
        }
        if self.presented_overview != overview_mode {
            self.presented_overview = overview_mode;
            let mode = match overview_mode {
                Some(true) => "alt-tab",
                Some(false) => "task-view",
                None => "closed",
            };
            let _ = door_sys::write(
                format!("DOOR_P12 OVERVIEW_PRESENTED mode={mode} frame={serial}\n").as_bytes(),
            );
        }
        Ok(())
    }
    fn paint_captions(&mut self, work: &mut FrameWork) -> Result<(), Status> {
        let frame = self
            .engine
            .repaint_captions()
            .map_err(|_| Status::IoFailure)?;
        let serial = frame.serial;
        let (width, height) = (frame.width, frame.height);
        let region = frame.damage.ok_or(Status::IoFailure)?;
        work.mark_composed(door_sys::time()?);
        let mut result = Ok(());
        let mut serviced = 0;
        for span in
            door_window::scanout::Spans::new(width, height, region).ok_or(Status::IoFailure)?
        {
            let output = self
                .engine
                .transfer_view(serial)
                .map_err(|_| Status::IoFailure)?;
            work.calls = work.calls.saturating_add(1);
            if let Err(error) = scanout(
                self.display,
                &mut self.presentation,
                (span.start / 4) as u64,
                &output.pixels[span.clone()],
            ) {
                result = Err(error);
                break;
            }
            work.bytes = work.bytes.saturating_add(span.len() as u64);
            serviced += span.len();
            if serviced >= 32 * 1024 {
                serviced = 0;
                self.input()?;
                if self.cursor_dirty {
                    self.present_cursor_on(Some(serial))?;
                }
            }
        }
        self.finish_transfer(serial, result.is_ok(), work)?;
        result?;
        self.present_cursor()
    }
    fn paint_icons(&mut self, work: &mut FrameWork) -> Result<(), Status> {
        let r = self.engine.output_rect();
        let scale = self.engine.scale();
        let area = self.taskbar_layout().work_area();
        let region = door_window::taskbar::physical(desktop::icon_column(area), scale);
        let icons = self.desktop_icons();
        let wallpaper = &self.wallpaper;
        let fonts = &self.fonts;
        let frame = self
            .engine
            .repaint_overlay(region, |canvas| {
                let stride = r.width() as usize * 4;
                for y in region.y() as usize..region.bottom() as usize {
                    let at = y * stride + region.x() as usize * 4;
                    let len = region.width() as usize * 4;
                    canvas.pixels_mut()[at..at + len].copy_from_slice(&wallpaper[at..at + len]);
                }
                canvas.set_clip(region);
                let _ = canvas.set_scale(scale);
                if let Some(icons) = icons.as_ref() {
                    desktop::paint_icons(canvas, fonts, icons);
                }
                let _ = canvas.set_scale(g::Scale::ONE);
            })
            .map_err(|_| Status::IoFailure)?;
        let serial = frame.serial;
        work.mark_composed(door_sys::time()?);
        self.icons_dirty = false;
        let mut transferred = Ok(());
        let mut serviced = 0;
        for span in door_window::scanout::Spans::new(r.width(), r.height(), region)
            .ok_or(Status::IoFailure)?
        {
            let output = self
                .engine
                .transfer_view(serial)
                .map_err(|_| Status::IoFailure)?;
            work.calls = work.calls.saturating_add(1);
            if let Err(error) = scanout(
                self.display,
                &mut self.presentation,
                (span.start / 4) as u64,
                &output.pixels[span.clone()],
            ) {
                transferred = Err(error);
                break;
            }
            work.bytes = work.bytes.saturating_add(span.len() as u64);
            serviced += span.len();
            if serviced >= 32 * 1024 {
                serviced = 0;
                self.input()?;
                if self.cursor_dirty {
                    self.present_cursor_on(Some(serial))?;
                }
            }
        }
        self.finish_transfer(serial, transferred.is_ok(), work)?;
        transferred?;
        self.present_cursor()
    }
    /// Repaint only shell regions over the retained scene cache: the bar
    /// alone for hover changes, plus open/animating surfaces, menus and the
    /// snap bar, and whatever the previous shell frame covered (dismissal).
    fn paint_bar(&mut self, work: &mut FrameWork) -> Result<(), Status> {
        self.sync_shell();
        let scene = self.capture_overlay_scene();
        let r = self.engine.output_rect();
        let scale = self.engine.scale();
        let layout = self.taskbar_layout();
        let now = self.shell_now();
        let mut logical = desktop::regions(&self.shell, &self.tray, layout, now, &self.fonts);
        let current = logical.clone();
        for previous in &self.shell.presented {
            if !logical.contains(previous) {
                logical.push(*previous);
            }
        }
        let regions: Vec<g::Rect> = logical
            .iter()
            .filter_map(|l| door_window::taskbar::physical(*l, scale).intersection(r))
            .collect();
        let Some(bounds) = regions.iter().copied().reduce(door_window::paint::union) else {
            self.bar_dirty = false;
            return Ok(());
        };
        let cached = door_window::taskbar::physical(layout.cache(), scale);
        let buttons = self.taskbar_buttons();
        let extras = self.taskbar_extras();
        let marks = self.taskbar_marks();
        let task_view = self.overview.as_ref().is_some_and(|v| !v.is_switcher());
        let base = &self.overlay_underlay;
        let fonts = &self.fonts;
        let pointer = (scale.logical(self.x), scale.logical(self.y));
        let dark = self.dark;
        let material = self.taskbar_material.pixels();
        let shell = &mut self.shell;
        let tray = &mut self.tray;
        let frame = self
            .engine
            .repaint_overlay(bounds, |canvas| {
                for region in &regions {
                    restore_cache(
                        canvas.pixels_mut(),
                        r.width() as usize * 4,
                        base,
                        cached,
                        *region,
                    );
                }
                canvas.set_clip(bounds);
                let _ = canvas.set_scale(scale);
                desktop::overlay(
                    canvas,
                    desktop::Overlay {
                        fonts,
                        layout,
                        buttons: &buttons,
                        extras: &extras,
                        marks: &marks,
                        shell,
                        tray,
                        pointer,
                        dark,
                        material,
                        underlay: base,
                        now,
                        task_view,
                        rebuild: false,
                    },
                );
                let _ = canvas.set_scale(g::Scale::ONE);
            })
            .map_err(|_| Status::IoFailure)?;
        let serial = frame.serial;
        work.mark_composed(door_sys::time()?);
        let mut result = Ok(());
        self.bar_dirty = false;
        let mut serviced = 0;
        'regions: for region in &regions {
            for span in door_window::scanout::Spans::new(r.width(), r.height(), *region)
                .ok_or(Status::IoFailure)?
            {
                let output = self
                    .engine
                    .transfer_view(serial)
                    .map_err(|_| Status::IoFailure)?;
                let bytes = &output.pixels[span.clone()];
                work.calls = work.calls.saturating_add(1);
                if let Err(error) = scanout(
                    self.display,
                    &mut self.presentation,
                    (span.start / 4) as u64,
                    bytes,
                ) {
                    result = Err(error);
                    break 'regions;
                }
                work.bytes = work.bytes.saturating_add(bytes.len() as u64);
                serviced += bytes.len();
                if serviced >= 32 * 1024 {
                    serviced = 0;
                    self.input()?;
                    if self.cursor_dirty {
                        self.present_cursor_on(Some(serial))?;
                    }
                }
            }
        }
        self.finish_transfer(serial, result.is_ok(), work)?;
        result?;
        self.shell.presented = current;
        self.overlay_scene_presented(scene);
        if !self.bar_reported
            && !layout.vertical()
            && regions.len() == 1
            && regions[0].height() == door_window::taskbar::physical(layout.bar(), scale).height()
        {
            let rows = regions[0].height();
            let _ =
                door_sys::write(format!("DOOR_P12 BAR FAST transferred_rows={rows}\n").as_bytes());
            self.bar_reported = true;
        }
        self.present_cursor()
    }
    pub(super) fn present_cursor(&mut self) -> Result<(), Status> {
        self.present_cursor_on(None)
    }
    fn present_cursor_on(&mut self, serial: Option<u64>) -> Result<(), Status> {
        let icon = if self.session.is_none() || self.launcher || self.overlay_contains() {
            g::CursorIcon::Arrow
        } else if let Some(drag) = self.drag {
            g::CursorIcon::resize(drag.edges)
        } else if let Some(id) = self.engine.captured_window() {
            self.engine
                .window_info(id)
                .map_or(g::CursorIcon::Arrow, |w| w.cursor)
        } else if let Some((id, region)) = self.engine.hit_test(self.x, self.y) {
            self.engine
                .window_info(id)
                .map_or(g::CursorIcon::Arrow, |w| match region {
                    g::HitRegion::Client => w.cursor,
                    g::HitRegion::Resize if w.state == g::WindowState::Normal => {
                        g::CursorIcon::resize(g::CursorIcon::resize_edges_scaled(
                            w.outer,
                            self.x,
                            self.y,
                            self.engine.scale(),
                        ))
                    }
                    _ => g::CursorIcon::Arrow,
                })
        } else {
            g::CursorIcon::Arrow
        };
        let scale = self.engine.scale();
        let size = g::CursorIcon::size(scale);
        let (hx, hy) = icon.hotspot_scaled(scale);
        let (px, py) = self.displayed_pointer();
        let origin = (px - hx, py - hy);
        let output = match serial {
            Some(serial) => self.engine.transfer_view(serial),
            None => self.engine.output_view(),
        }
        .map_err(|_| Status::IoFailure)?;
        // Under Magnifier the cursor plane restores from the magnified frame.
        let magnified = self.shell.magnifier.is_some()
            && self.shell.magnified.len() == output.width as usize * output.height as usize * 4;
        let (source, stride) = if magnified {
            (&self.shell.magnified[..], output.width as usize * 4)
        } else {
            (output.pixels, output.stride_bytes)
        };
        // Emulated cursor plane: restore the prior small rectangle from the
        // immutable composed image, then paint the new cursor over a local copy.
        for (x, y, paint) in self
            .last_cursor
            .map(|p| (p.0, p.1, false))
            .into_iter()
            .chain(core::iter::once((origin.0, origin.1, true)))
        {
            let left = x.max(0) as usize;
            let top = y.max(0) as usize;
            let right = (x + size as i32).clamp(0, output.width as i32) as usize;
            let bottom = (y + size as i32).clamp(0, output.height as i32) as usize;
            let width = right.saturating_sub(left);
            for py in top..bottom {
                let offset = py * stride + left * 4;
                let mut row = [0; 384];
                row[..width * 4].copy_from_slice(&source[offset..offset + width * 4]);
                if paint {
                    for dx in 0..width {
                        if let Some(pixel) =
                            icon.pixel_scaled((left + dx) as i32 - x, py as i32 - y, scale)
                        {
                            for c in 0..3 {
                                let old = row[dx * 4 + c];
                                row[dx * 4 + c] = if pixel[3] == 0 {
                                    255 - old
                                } else {
                                    (u32::from(pixel[c])
                                        + (u32::from(old) * u32::from(255 - pixel[3]) + 127) / 255)
                                        .min(255) as u8
                                };
                            }
                            row[dx * 4 + 3] = 255;
                        }
                    }
                }
                if width != 0 {
                    scanout(
                        self.display,
                        &mut self.presentation,
                        (py * output.width as usize + left) as u64,
                        &row[..width * 4],
                    )?;
                }
            }
        }
        self.last_cursor = Some(origin);
        self.cursor_dirty = false;
        Ok(())
    }
}
