//! Trusted shortcuts and overview dispatch share the engine's placement, focus
//! and workspace records. No app can synthesize these global input actions.
use super::*;
use desktop::App;
use door_window::{
    overview::{Action, Overview},
    policy::SystemCommand as C,
};
impl Desktop {
    pub(super) fn task_switch(&mut self, backwards: bool) {
        self.shell_focus = None;
        if let Some(view) = self.overview.as_mut().filter(|v| v.is_switcher()) {
            view.step(backwards);
        } else {
            self.overview_focus = self.engine.focused_window();
            let mut view = Overview::switcher(&self.engine, backwards);
            // Alt+Tab headers show each window's app icon.
            view.set_icons(
                self.app_classes
                    .iter()
                    .map(|(owner, app)| (*owner, door_window::shell_view::app_icon(*app)))
                    .collect(),
            );
            self.overview = Some(view);
            self.launcher = false;
            self.tray.open = None;
            self.drag = None;
            self.caption_press = None;
            self.engine.clear_focus();
            self.manual_focus(None);
        }
        self.engine.invalidate();
    }
    pub(super) fn switch_desktop(&mut self, id: g::WorkspaceId) {
        self.shell_focus = None;
        let prior = (id == self.engine.active_workspace())
            .then_some(self.overview_focus)
            .flatten();
        if self.engine.switch_workspace(id).is_err() {
            return;
        }
        if let Some(prior) =
            prior.filter(|id| self.engine.window_info(*id).is_ok_and(|w| w.visible))
        {
            let _ = self.engine.activate(prior);
        }
        self.overview_focus = None;
        self.drag = None;
        self.caption_press = None;
        self.desktop_press = None;
        self.desktop_selection.cancel_gesture();
        self.system_keys_cancel_capture();
        self.launcher = false;
        self.tray.open = None;
        self.overview = None;
        self.hidden_windows.clear();
        self.manual_focus(self.engine.focused_window());
        let _ = door_sys::write(
            format!(
                "DOOR_P12 WORKSPACE active={} count={}\n",
                id.0,
                self.engine.workspace_ids().count()
            )
            .as_bytes(),
        );
    }
    fn system_keys_cancel_capture(&mut self) {
        self.overlay_buttons = 0;
    }
    pub(super) fn toggle_desktop(&mut self) {
        self.shell_focus = None;
        if self.hidden_windows.is_empty() {
            let ids: Vec<_> = self
                .engine
                .windows()
                .filter(|w| {
                    w.workspace == self.engine.active_workspace()
                        && w.role == g::Role::TopLevel
                        && matches!(w.state, g::WindowState::Normal | g::WindowState::Maximized)
                })
                .map(|w| (w.id, w.owner))
                .collect();
            for (id, owner) in ids {
                if self
                    .engine
                    .request_state(owner, id, g::WindowState::Minimized)
                    .is_ok()
                {
                    self.hidden_windows.push(id);
                }
            }
        } else {
            let ids = core::mem::take(&mut self.hidden_windows);
            for id in ids {
                if let Ok(w) = self.engine.window_info(id) {
                    let owner = w.owner;
                    if w.workspace == self.engine.active_workspace()
                        && w.state == g::WindowState::Minimized
                    {
                        let _ = self.engine.restore_window(owner, id);
                        let _ = self.engine.activate(id);
                    }
                }
            }
        }
        self.engine.invalidate();
        self.manual_focus(self.engine.focused_window());
    }
    pub(super) fn system_command(&mut self, command: C) {
        match command {
            C::QuickSettings | C::Notifications => {
                self.remember_shell_focus();
                self.close_menu();
                self.tray.backdrop.clear();
                self.launcher = false;
                self.overview = None;
                self.drag = None;
                self.caption_press = None;
                self.desktop_press = None;
                self.desktop_selection.cancel_gesture();
                self.system_keys_cancel_capture();
                self.tray.activate(if command == C::QuickSettings {
                    door_window::tray::Action::Devices
                } else {
                    door_window::tray::Action::Clock
                });
                if self.tray.open.is_some() {
                    self.engine.clear_focus();
                    self.manual_focus(None);
                } else {
                    self.restore_shell_focus();
                }
                self.engine.invalidate();
            }
            C::Start => {
                self.overview = None;
                self.toggle_start(false);
            }
            C::Search => {
                self.overview = None;
                self.toggle_start(true);
            }
            C::Files => {
                self.overview = None;
                self.close_start();
                self.close_menu();
                self.engine.invalidate();
                self.open_app(App::Files);
            }
            C::Settings => {
                self.overview = None;
                self.close_start();
                self.close_menu();
                self.engine.invalidate();
                self.open_app(App::Settings);
            }
            C::QuickLink => {
                self.overview = None;
                self.open_quick_link();
            }
            C::Run => self.open_run(),
            C::Cast | C::Project => {
                self.overview = None;
                self.close_start();
                self.close_menu();
                self.desktop_selection.cancel_gesture();
                self.system_keys_cancel_capture();
                let page = if command == C::Cast {
                    door_window::tray::QuickPage::Cast
                } else {
                    door_window::tray::QuickPage::Project
                };
                if self.tray.open == Some(door_window::tray::Flyout::Devices)
                    && self.tray.quick_page == page
                {
                    self.tray.open = None;
                    self.restore_shell_focus();
                } else {
                    if self.tray.open != Some(door_window::tray::Flyout::Devices) {
                        self.tray.activate(door_window::tray::Action::Devices);
                    }
                    self.tray.quick_page = page;
                    self.engine.clear_focus();
                    self.manual_focus(None);
                }
                self.engine.invalidate();
            }
            C::ClipboardHistory => self.toggle_clipboard_panel(),
            // Win+Shift+S and Print Screen: the snip overlay.
            C::Snip => self.start_snip(door_window::snip::Mode::Rectangle, 0, None),
            // Written through to the user's profile like Settings: Init
            // stores it, then SET_DESKTOP_CONTROLS applies it everywhere.
            C::ToggleContrast => {
                self.shell.controls_pending = Some(self.controls.toggle_contrast());
            }
            // DISPLAY-01, test image only: fail the next checked copies so
            // the presentation fallback runs (a full repaint is forced).
            C::PresentFault => {
                let copies = door_window::scanout::INJECTED_COPIES;
                if self.presentation.arm(copies) {
                    self.engine.invalidate();
                    door_rt::diagnostic(format_args!(
                        "DOOR_P12 PRESENT_FAULT armed copies={copies}
"
                    ));
                }
            }
            C::Lock => self.lock(),
            C::MinimizeAll => {
                self.overview = None;
                self.close_start();
                self.close_menu();
                self.tray.open = None;
                self.minimize_all();
            }
            C::SnapLayouts => {
                self.close_start();
                self.close_menu();
                self.snap_layouts_for_focus();
            }
            C::Taskbar(n) => {
                self.overview = None;
                self.close_menu();
                self.taskbar_number(n as usize);
            }
            C::Desktop => {
                self.overview = None;
                self.close_menu();
                self.launcher = false;
                self.tray.open = None;
                self.toggle_desktop();
            }
            C::TaskView => {
                self.shell_focus = None;
                if self.overview.is_some() {
                    self.overview_action(Action::Dismiss);
                    return;
                }
                self.overview_focus = self.engine.focused_window();
                if self.overview.is_none() {
                    let mut view = Overview::default();
                    view.refresh(&self.engine);
                    view.select_window(self.engine.focused_window());
                    self.overview = Some(view);
                }
                self.launcher = false;
                self.tray.open = None;
                self.drag = None;
                self.caption_press = None;
                self.engine.clear_focus();
                self.manual_focus(None);
                self.engine.invalidate();
                let _ = door_sys::write(
                    format!(
                        "DOOR_P12 TASK_VIEW open={}\n",
                        u8::from(self.overview.is_some())
                    )
                    .as_bytes(),
                );
            }
            C::NewDesktop => {
                if let Ok(id) = self.engine.create_workspace() {
                    self.switch_desktop(id);
                }
            }
            C::CloseDesktop => {
                if let Ok(id) = self.engine.remove_workspace(self.engine.active_workspace()) {
                    self.switch_desktop(id);
                }
            }
            C::PreviousDesktop | C::NextDesktop => {
                let ids: Vec<_> = self.engine.workspace_ids().collect();
                let i = ids
                    .iter()
                    .position(|id| *id == self.engine.active_workspace())
                    .unwrap_or(0);
                let next = if command == C::PreviousDesktop {
                    i.saturating_sub(1)
                } else {
                    (i + 1).min(ids.len() - 1)
                };
                if next != i {
                    self.switch_desktop(ids[next]);
                }
            }
            C::Left | C::Right | C::Up | C::Down if !self.snap_off() => {
                let Some(w) = self
                    .engine
                    .focused_window()
                    .and_then(|id| self.engine.window_info(id).ok())
                else {
                    return;
                };
                let (id, owner, epoch, state, snapped) =
                    (w.id, w.owner, w.epoch, w.state, w.snapped);
                let result = match command {
                    C::Left => self.engine.snap_window(owner, id, g::Snap::Left, None),
                    C::Right => self.engine.snap_window(owner, id, g::Snap::Right, None),
                    C::Up => self.engine.snap_window(owner, id, g::Snap::Maximize, None),
                    _ if snapped => self.engine.unsnap_window(owner, id),
                    _ if state == g::WindowState::Maximized => {
                        self.engine.restore_window(owner, id)
                    }
                    _ => self
                        .engine
                        .request_state(owner, id, g::WindowState::Minimized),
                };
                if result.is_ok() && self.engine.window_info(id).is_ok_and(|w| w.epoch != epoch) {
                    self.attachments.retain(|a| a.window != id);
                }
                self.manual_focus(self.engine.focused_window());
                if result.is_ok() && matches!(command, C::Left | C::Right) {
                    self.assist_after_snap(
                        if command == C::Left {
                            g::Snap::Left
                        } else {
                            g::Snap::Right
                        },
                        id,
                    );
                }
            }
            // "Snap windows" off: Windows logo key + arrows do nothing.
            C::Left | C::Right | C::Up | C::Down => (),
        }
    }
    pub(super) fn overview_action(&mut self, action: Action) {
        match action {
            Action::Dismiss => {
                self.overview = None;
                let focus = self
                    .overview_focus
                    .take()
                    .filter(|id| self.engine.window_info(*id).is_ok_and(|w| w.visible))
                    .or_else(|| {
                        self.engine
                            .windows()
                            .filter(|w| w.visible && w.role == g::Role::TopLevel)
                            .last()
                            .map(|w| w.id)
                    });
                if let Some(id) = focus {
                    let _ = self.engine.activate(id);
                }
                self.manual_focus(self.engine.focused_window());
            }
            Action::Window(id) => {
                self.overview = None;
                self.activate_desktop_window(id);
                self.manual_focus(self.engine.focused_window());
            }
            Action::CloseWindow(id) => {
                if let Ok(w) = self.engine.window_info(id) {
                    let _ = self.close(w.owner, id);
                }
            }
            Action::Desktop(id) => self.switch_desktop(id),
            Action::NewDesktop => self.system_command(C::NewDesktop),
            Action::CloseDesktop(id) => {
                let active = self.engine.active_workspace();
                if let Ok(target) = self.engine.remove_workspace(id) {
                    if active == id {
                        self.switch_desktop(target);
                    }
                    let mut view = Overview::default();
                    view.refresh(&self.engine);
                    self.overview = Some(view);
                }
            }
        }
        self.engine.invalidate();
    }
    pub(super) fn overview_hit(&self) -> Option<Action> {
        let view = self.overview.as_ref()?;
        let scale = self.engine.scale();
        let out = self.engine.output_rect();
        if self
            .taskbar_layout()
            .bar()
            .contains(scale.logical(self.x), scale.logical(self.y))
        {
            return None;
        }
        Some(view.hit(
            scale.logical_len(out.width()),
            scale.logical_len(out.height()),
            &self.engine.workspace_ids().collect::<Vec<_>>(),
            scale.logical(self.x),
            scale.logical(self.y),
        ))
    }
}
