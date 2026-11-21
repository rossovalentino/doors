#![no_std]
#![forbid(unsafe_code)]
//! Door UI platform adapter. A door-ui application implements `App`; `run`
//! hosts it on the Door window server through the shared native runtime
//! (window creation, surfaces, input delivery, theme and controls events,
//! fonts from the shared asset pack). The adapter translates platform input
//! into `Ui` calls, renders damage into a private frame, and (with the
//! `accessibility` feature) publishes the semantic tree and serves
//! brokered assistive actions.
extern crate alloc;
use alloc::{string::String, vec::Vec};
use core::cell::RefCell;
use door_app_ui::{Application, View};
use door_graphics_core::Scale;
use door_graphics_text::FontAtlas;
use door_service_client::window::WindowClient;
use door_service_client::{Error, Storage};
use door_service_protocol::window as w;
use door_ui::{
    AtlasText, Fonts, Frame, KeyEvent, LPoint, LSize, Lu, Modifiers, Notification, Theme, Ui,
};
use door_workspace::runtime::WindowOptions;

/// Height of the caption strip the server's caption buttons sit in.
pub const CAPTION: u32 = 32;

/// A door-ui application.
pub trait App {
    fn title(&self) -> &str;
    /// Initial client size in logical units.
    fn size(&self) -> (u32, u32) {
        (1040, 720)
    }
    /// Build the widget tree (called once, before the first frame).
    fn build(&mut self, ui: &mut Ui);
    /// React to a control notification (update models with `ui.edit`).
    fn notify(&mut self, _ui: &mut Ui, _n: Notification) {}
    /// Periodic work on the UI thread (milliseconds clock).
    fn tick(&mut self, _ui: &mut Ui, _now: u64) {}
    /// The theme to render with, given the session theme (an engineering
    /// fixture such as the gallery may pin another one).
    fn theme(&self, system: Theme) -> Theme {
        system
    }
    /// Diagnostic marker hook (the gallery prints serial markers).
    fn trace(&mut self, _ui: &mut Ui) {}
    /// Platform services on the window connection: called with each
    /// door.accessibility event addressed to the app, then once with `None`
    /// per update (assistive clients read trees here).
    fn platform(&mut self, _ui: &mut Ui, _client: &mut WindowClient, _event: Option<w::Event>) {}
    /// HELLO capabilities beyond the adapter's own (an assistive client
    /// asks for `accessibility::CAP_ASSISTIVE`; the server decides).
    fn capabilities(&self) -> u32 {
        0
    }
}

/// The atlas key of the shared asset pack for a display percent.
pub fn atlas_key(percent: u32) -> Option<&'static str> {
    Some(match percent {
        100 => "fonts/segoe-ui/hinted-1x.dfa",
        125 => "fonts/segoe-ui/hinted-125.dfa",
        150 => "fonts/segoe-ui/hinted-150.dfa",
        175 => "fonts/segoe-ui/hinted-175.dfa",
        200 => "fonts/segoe-ui/hinted-2x.dfa",
        300 => "fonts/segoe-ui/hinted-3x.dfa",
        _ => return None,
    })
}
/// The hinted UI atlas for `percent`, from the shared asset pack (built-in
/// on host tools), with the process's Unicode engine attached.
pub fn atlas(percent: u32) -> Option<FontAtlas> {
    #[cfg(not(target_os = "none"))]
    let mut atlas = match percent % 100 {
        0 => FontAtlas::windows_reference_scaled(percent / 100).ok()?,
        _ => FontAtlas::from_packed_percent(
            door_workspace::assets::get(atlas_key(percent)?),
            percent,
        )
        .ok()?,
    };
    #[cfg(target_os = "none")]
    let mut atlas =
        FontAtlas::from_packed_percent(door_workspace::assets::get(atlas_key(percent)?), percent)
            .ok()?;
    if let Some(system) = door_workspace::assets::text_system() {
        atlas.attach_unicode(system, door_graphics_text::Family::Ui);
    }
    Some(atlas)
}

struct State {
    ui: Option<Ui>,
    fonts: Fonts,
    frame: Vec<u8>,
    physical: (u32, u32),
    zoom: Scale,
    pointer: LPoint,
    release: bool,
    platform: Vec<w::Event>,
    #[cfg(feature = "accessibility")]
    published: Option<u32>,
    #[cfg(feature = "accessibility")]
    actions: Vec<(u32, u64, u32)>,
}

/// `door_app_ui::Application` for a door-ui `App`.
pub struct Host<A: App> {
    app: A,
    title: String,
    state: RefCell<State>,
}
impl<A: App> Host<A> {
    pub fn new(app: A) -> Self {
        Self {
            title: app.title().into(),
            app,
            state: RefCell::new(State {
                ui: None,
                fonts: Fonts::new(),
                frame: Vec::new(),
                physical: (0, 0),
                zoom: Scale::ONE,
                pointer: LPoint::default(),
                release: false,
                platform: Vec::new(),
                #[cfg(feature = "accessibility")]
                published: None,
                #[cfg(feature = "accessibility")]
                actions: Vec::new(),
            }),
        }
    }
    pub fn app(&self) -> &A {
        &self.app
    }
    /// Run `f` with the built `Ui`, then deliver notifications to the app.
    fn with_ui<R>(&mut self, f: impl FnOnce(&mut Ui) -> R) -> R {
        let state = self.state.get_mut();
        if state.ui.is_none() {
            let (w, h) = self.app.size();
            let text = atlas(100).map(|a| AtlasText::new(a, 100));
            let mut ui = match text {
                Some(t) => Ui::new(
                    LSize::px(w as i32, h as i32),
                    Theme::light(),
                    alloc::boxed::Box::new(t),
                ),
                None => Ui::new(
                    LSize::px(w as i32, h as i32),
                    Theme::light(),
                    alloc::boxed::Box::new(NoText),
                ),
            };
            ui.set_title(&self.title);
            let root = ui.root();
            ui.style_mut(root, |s| s.padding.top = Lu::px(CAPTION as i32));
            self.app.build(&mut ui);
            state.ui = Some(ui);
        }
        let ui = state.ui.as_mut().unwrap();
        if state.release {
            state.release = false;
            ui.pointer_up(
                state.pointer,
                door_ui::MouseButton::Primary,
                Modifiers::NONE,
            );
        }
        let r = f(ui);
        for n in ui.take_notifications() {
            self.app.notify(ui, n);
        }
        self.app.trace(ui);
        r
    }
    fn needs_frame(&mut self) -> bool {
        self.state
            .get_mut()
            .ui
            .as_ref()
            .is_some_and(Ui::needs_frame)
    }
}

/// Measures nothing (only when no font atlas could be loaded at all).
struct NoText;
impl door_ui::TextSystem for NoText {
    fn measure(&self, _: &str, _: door_ui::TextRole) -> LSize {
        LSize::ZERO
    }
    fn line_height(&self, _: door_ui::TextRole) -> Lu {
        Lu::px(20)
    }
    fn baseline(&self, _: door_ui::TextRole) -> Lu {
        Lu::px(15)
    }
    fn ink(&self, _: door_ui::TextRole) -> (Lu, Lu) {
        (Lu::px(5), Lu::px(13))
    }
    fn caret_stops(&self, t: &str, _: door_ui::TextRole) -> Vec<(usize, Lu)> {
        alloc::vec![(0, Lu::ZERO), (t.len(), Lu::ZERO)]
    }
    fn text_scale(&self) -> u32 {
        100
    }
}

impl<A: App> Application for Host<A> {
    fn title(&self) -> &str {
        &self.title
    }
    fn capabilities(&self) -> u32 {
        #[cfg(feature = "accessibility")]
        return door_service_protocol::accessibility::CAP_ACCESSIBILITY | self.app.capabilities();
        #[cfg(not(feature = "accessibility"))]
        self.app.capabilities()
    }
    /// With `accessibility` the adapter publishes the toolkit's own tree
    /// (below); without it the runtime publishes a window-only tree.
    fn publishes_semantics(&self) -> bool {
        cfg!(feature = "accessibility")
    }
    fn options(&self) -> WindowOptions {
        let (width, height) = self.app.size();
        WindowOptions {
            width,
            height,
            caption_height: CAPTION,
            caption_left: 0,
            centered: false,
        }
    }
    fn configure(&mut self, width: u32, height: u32) {
        self.with_ui(|ui| ui.resize(LSize::px(width as i32, height as i32)));
    }
    fn display(&mut self, _w: u32, _h: u32, percent: u32) {
        let state = self.state.get_mut();
        if let Some(zoom) = Scale::from_percent(percent) {
            state.zoom = zoom;
            if let Some(a) = atlas(percent) {
                state.fonts.install(a);
            }
        }
    }
    fn draw(&self, v: &mut View<'_>) {
        let mut guard = self.state.borrow_mut();
        let state = &mut *guard;
        let Some(ui) = state.ui.as_mut() else { return };
        let controls = door_service_client::window::desktop_controls();
        ui.set_theme(
            self.app
                .theme(Theme::from_controls(v.palette.dark, controls)),
        );
        let zoom = v.zoom;
        let (pw, ph) = (zoom.len(v.width), zoom.len(v.height));
        if pw == 0 || ph == 0 {
            return;
        }
        let full = state.physical != (pw, ph) || state.zoom != zoom;
        if full {
            state.frame.clear();
            state.frame.resize(pw as usize * ph as usize * 4, 0);
            state.physical = (pw, ph);
            state.zoom = zoom;
        }
        if let Some(mut frame) = Frame::new(&mut state.frame, pw, ph, pw as usize * 4) {
            ui.render(&mut frame, zoom, &mut state.fonts, full);
        }
        v.color_image_physical(0, 0, pw, ph, &state.frame);
        let fg = if v.palette.dark {
            [255, 255, 255, 255]
        } else {
            [26, 26, 26, 255]
        };
        v.text(
            12,
            8,
            &self.title,
            door_graphics_text::TextStyle::Regular12,
            fg,
        );
    }
    fn target(&self, _x: i32, _y: i32) -> Option<u32> {
        // Every client point is a door-ui target; the release position
        // arrives through `click_at`.
        Some(1)
    }
    fn click(&mut self, _id: u32, _s: &mut Storage) -> Result<bool, Error> {
        Ok(false)
    }
    fn click_at(
        &mut self,
        _id: u32,
        x: i32,
        y: i32,
        _s: &mut Storage,
        _f: &FontAtlas,
    ) -> Result<bool, Error> {
        let p = LPoint::px(x, y);
        self.state.get_mut().release = false;
        self.with_ui(|ui| ui.pointer_up(p, door_ui::MouseButton::Primary, Modifiers::NONE));
        Ok(true)
    }
    fn pointer_down(&mut self, x: i32, y: i32, _f: &FontAtlas) -> bool {
        let p = LPoint::px(x, y);
        self.state.get_mut().pointer = p;
        self.with_ui(|ui| ui.pointer_down(p, door_ui::MouseButton::Primary, Modifiers::NONE));
        true
    }
    fn pointer_motion(&mut self, x: i32, y: i32, _f: &FontAtlas) -> bool {
        let p = LPoint::px(x, y);
        self.state.get_mut().pointer = p;
        self.with_ui(|ui| ui.pointer_move(p, Modifiers::NONE));
        self.needs_frame()
    }
    fn pointer_up(&mut self) {
        // Completed by `click_at` (inside) or at the next callback.
        self.state.get_mut().release = true;
    }
    fn wheel(&mut self, x: i32, y: i32, notches: i32, _f: &FontAtlas) -> bool {
        self.with_ui(|ui| ui.wheel(LPoint::px(x, y), notches, false, Modifiers::NONE));
        self.needs_frame()
    }
    fn context(&mut self, x: i32, y: i32, _w: u32, _h: u32) {
        let p = LPoint::px(x, y);
        self.with_ui(|ui| {
            ui.pointer_down(p, door_ui::MouseButton::Secondary, Modifiers::NONE);
            ui.pointer_up(p, door_ui::MouseButton::Secondary, Modifiers::NONE);
        });
    }
    fn keyboard_context(&mut self, _w: u32, _h: u32) {
        self.with_ui(|ui| ui.key(door_ui::Key::Menu, Modifiers::NONE));
    }
    fn key(&mut self, key: u32, _s: &mut Storage) -> Result<bool, Error> {
        self.with_ui(|ui| ui.key_down(KeyEvent::from_door(key)));
        Ok(true)
    }
    fn focus(&mut self, focused: bool) {
        self.with_ui(|ui| ui.window_focus(focused));
    }
    fn modal(&self) -> bool {
        false
    }
    fn menu_open(&self) -> bool {
        self.state
            .borrow()
            .ui
            .as_ref()
            .is_some_and(|ui| !ui.overlays().is_empty())
    }
    fn dismiss_menu(&mut self) {
        self.with_ui(|ui| {
            ui.capture_lost();
        });
    }
    fn text_hit(&self, _x: i32, _y: i32) -> bool {
        self.state
            .borrow()
            .ui
            .as_ref()
            .is_some_and(|ui| ui.cursor() == door_ui::Cursor::Text)
    }
    fn event(&mut self, e: w::Event) -> Result<bool, Error> {
        match e.kind {
            // Key releases (buttons activate on Space up).
            w::INPUT if e.detail == 1 && e.b == 0 => {
                self.with_ui(|ui| ui.key_up(KeyEvent::from_door(e.a)));
                Ok(true)
            }
            w::APPEARANCE_CHANGED | w::CONTROLS_CHANGED => Ok(true),
            k if (0x120..=0x12f).contains(&k) && k != 0x122 => {
                self.state.get_mut().platform.push(e);
                Ok(false)
            }
            #[cfg(feature = "accessibility")]
            k if k == door_service_protocol::accessibility::ACTION => {
                self.state.get_mut().actions.push((e.b, e.value, e.a));
                Ok(false)
            }
            _ => Ok(false),
        }
    }
    fn tick(&mut self, _s: &mut Storage, now: u64) -> Result<bool, Error> {
        let wants = self.state.get_mut().ui.as_ref().is_some_and(Ui::wants_tick);
        self.with_ui(|ui| {
            if wants {
                ui.tick(now);
            } else {
                ui.set_time(now);
            }
        });
        let ui = self.state.get_mut().ui.as_mut().unwrap();
        self.app.tick(ui, now);
        Ok(self.needs_frame())
    }
    #[allow(unused_variables)]
    fn update(
        &mut self,
        _s: &mut Storage,
        client: &mut WindowClient,
        window: u64,
    ) -> Result<bool, Error> {
        {
            let state = self.state.get_mut();
            if let Some(ui) = state.ui.as_mut() {
                for e in core::mem::take(&mut state.platform) {
                    self.app.platform(ui, client, Some(e));
                }
                self.app.platform(ui, client, None);
                for n in ui.take_notifications() {
                    self.app.notify(ui, n);
                }
            }
        }
        #[cfg(feature = "accessibility")]
        {
            use door_service_protocol::{Code, accessibility as a};
            if client.supports_accessibility() {
                let state = self.state.get_mut();
                let Some(ui) = state.ui.as_mut() else {
                    return Ok(false);
                };
                ui.set_window_id(window);
                for (serial, node, code) in core::mem::take(&mut state.actions) {
                    let mut arg = [0u8; a::ARG_BYTES];
                    let result = match client.accessibility_action(serial, &mut arg) {
                        Ok(req) => {
                            match door_accessibility::Action::from_code(req.action, &arg[..req.len])
                            {
                                Some(action) if req.node == node && req.action == code => {
                                    match ui.accessibility_action(
                                        door_accessibility::NodeId(node),
                                        action,
                                    ) {
                                        Ok(()) => a::RESULT_DONE,
                                        Err(_) => a::RESULT_REJECTED,
                                    }
                                }
                                _ => a::RESULT_REJECTED,
                            }
                        }
                        Err(_) => continue,
                    };
                    let _ = client.accessibility_action_done(serial, result);
                }
                if let Some(update) = ui.take_semantics() {
                    // A delta only applies to the version the broker holds.
                    let delta = update
                        .delta
                        .filter(|_| state.published.is_some() && state.published == update.base);
                    let sent = match delta {
                        Some(d) => match client.accessibility_publish(window, &d) {
                            Err(Error::Service(Code::Changed)) => {
                                client.accessibility_publish(window, &update.full)
                            }
                            other => other,
                        },
                        None => client.accessibility_publish(window, &update.full),
                    };
                    state.published = sent.ok();
                }
            }
        }
        Ok(self.needs_frame())
    }
}

/// Run a door-ui application until its window closes.
pub fn run<A: App>(startup: &door_abi::platform::StartupV2, app: A) -> Result<(), Error> {
    let mut host = Host::new(app);
    door_app_ui::run(startup, &mut host)
}
