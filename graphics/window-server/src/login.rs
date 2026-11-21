//! Trusted sign-in and lock surface. Session admission and unlock occur only
//! after the credential gate below accepts a password or PIN checked by the
//! session service; ordinary clients have no access to this surface.
use door_graphics_core::{Canvas, Rect};
use door_graphics_text::{Clip, FontAtlas, Target, TextRun, TextStyle};
/// Fixed logical metrics from the retained Windows sign-in capture. Painting
/// and trusted input use the same button, including at native 200% density.
pub struct Layout {
    pub avatar: Rect,
    pub button: Rect,
    pub name_top: i32,
}
impl Layout {
    pub fn new(width: u32, height: u32) -> Self {
        let center = width as i32 / 2;
        let middle = height as i32 / 2;
        Self {
            avatar: Rect::new(center - 96, middle - 228, 192, 192).unwrap(),
            button: Rect::new(center - 62, middle + 24, 124, 36).unwrap(),
            name_top: middle - 12,
        }
    }
}
pub fn draw(canvas: &mut Canvas<'_>, fonts: &FontAtlas, pending: bool) {
    let (width, height, _) = canvas.dimensions();
    let scale = canvas.scale();
    let center = width as i32 / 2;
    let layout = Layout::new(width, height);
    let _ = canvas.fill_rect(Rect::new(0, 0, width, height).unwrap(), [30, 14, 4, 130]);
    let _ = canvas.fill_rounded_rect(layout.avatar, 96, [18, 18, 18, 18]);
    let mask: &[u8] = if scale.asset_factor() == 3 {
        include_bytes!("../../../assets/icons/windows-shell/contact.288.mask")
    } else if scale.asset_factor() == 2 {
        include_bytes!("../../../assets/icons/windows-shell/contact.192.mask")
    } else {
        include_bytes!("../../../assets/icons/windows-shell/contact.96.mask")
    };
    if canvas
        .clip_rect()
        .and_then(|clip| clip.intersection(crate::taskbar::physical(layout.avatar, scale)))
        .is_some()
    {
        crate::tray::glyph(
            canvas,
            center - 48,
            layout.avatar.y() + 48,
            96,
            mask,
            [255; 4],
        );
    }
    let Some(clip) = canvas.clip_rect() else {
        return;
    };
    let (pw, ph, stride) = canvas.physical_dimensions();
    if let Ok(mut target) = Target::new(canvas.pixels_mut(), pw, ph, stride) {
        let title = "Local account";
        let width = fonts
            .measure(title, TextStyle::Semibold28)
            .map_or(0, |metrics| metrics.advance_width);
        let _ = fonts.draw_physical(
            &mut target,
            Clip::new(clip.x(), clip.y(), clip.width(), clip.height()),
            TextRun {
                text: title,
                x: scale.px(center - width as i32 / 2),
                top: scale.px(layout.name_top),
                style: TextStyle::Semibold28,
                color: [255; 4],
            },
        );
    }
    let _ = canvas.fill_rounded_rect(layout.button, 4, [46, 46, 46, 46]);
    let Some(clip) = canvas.clip_rect() else {
        return;
    };
    let (pw, ph, stride) = canvas.physical_dimensions();
    if let Ok(mut target) = Target::new(canvas.pixels_mut(), pw, ph, stride) {
        let title = if pending { "Signing in..." } else { "Sign in" };
        let width = fonts
            .measure(title, TextStyle::Regular14)
            .map_or(0, |metrics| metrics.advance_width);
        let _ = fonts.draw_physical(
            &mut target,
            Clip::new(clip.x(), clip.y(), clip.width(), clip.height()),
            TextRun {
                text: title,
                x: scale.px(center - width as i32 / 2),
                top: scale.px(layout.button.y() + 9),
                style: TextStyle::Regular14,
                color: [255; 4],
            },
        );
    }
}

// ---------------------------------------------------------------------------
// Credential gate (P14-LOGON, ADR 0020). The trusted sign-in and lock surface
// owns only the typing in progress; the session service owns the Argon2id
// records and the throttle. `Layout`/`draw` above remain the passwordless card
// until every caller uses the gate. With a password and a PIN both enrolled
// the gate asks for the PIN (as Windows) and "Sign-in options" switches to
// the password; with one kind it behaves exactly as before two kinds existed.
// ---------------------------------------------------------------------------
use door_keyboard_layouts::{Translator, layouts};
use door_service_protocol::credential::{self, Kind, SECRET_MAX, State, Verdict};

/// Why the gate is shown.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Purpose {
    SignIn,
    Unlock,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Page {
    /// Waiting for the credential state.
    Loading,
    /// First run: create a password or PIN (and confirm it).
    Enroll,
    SignIn,
    /// The credential service is unreachable or its record is damaged. The
    /// gate fails closed: there is no passwordless fallback.
    Unavailable,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Notice {
    None,
    Incorrect,
    Mismatch,
    Policy,
    Throttled,
    Working,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Action {
    None,
    Redraw,
    /// The secret was accepted (or enrolled): sign in or unlock now.
    Granted,
}
/// A request the gate asks its backend to begin.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Request<'a> {
    State,
    Enroll(Kind, &'a [u8]),
    Verify(Kind, &'a [u8]),
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Polled {
    Pending,
    State(State),
    Enrolled(State),
    Verdict(Verdict),
    Failed,
}
/// The session service connection (production) or a host fake (tests).
pub trait Backend {
    /// Begins one request; false when it could not be sent.
    fn begin(&mut self, request: Request<'_>, now: u64) -> bool;
    /// The local accounts (None: only the first, or unknown).
    fn accounts(&mut self) -> Option<door_service_protocol::credential::Accounts> {
        None
    }
    /// Whose credentials the next requests check; false when refused.
    fn select(&mut self, _uid: u32) -> bool {
        true
    }
    fn poll(&mut self, now: u64) -> Polled;
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Secret {
    bytes: [u8; SECRET_MAX],
    len: u8,
}
impl core::fmt::Debug for Secret {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "Secret({} chars)", self.chars())
    }
}
impl Secret {
    pub const EMPTY: Self = Self {
        bytes: [0; SECRET_MAX],
        len: 0,
    };
    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes[..usize::from(self.len)]
    }
    pub fn chars(&self) -> usize {
        core::str::from_utf8(self.as_bytes()).map_or(0, |s| s.chars().count())
    }
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
    fn push(&mut self, kind: Kind, ch: char) -> bool {
        let mut utf8 = [0; 4];
        let encoded = ch.encode_utf8(&mut utf8).as_bytes();
        let len = usize::from(self.len);
        if !kind.accepts(ch) || len + encoded.len() > kind.max_len() {
            return false;
        }
        self.bytes[len..len + encoded.len()].copy_from_slice(encoded);
        self.len += encoded.len() as u8;
        true
    }
    fn pop(&mut self) -> bool {
        let Ok(text) = core::str::from_utf8(self.as_bytes()) else {
            self.clear();
            return true;
        };
        let Some(last) = text.chars().next_back() else {
            return false;
        };
        let len = usize::from(self.len) - last.len_utf8();
        self.bytes[len..].fill(0);
        self.len = len as u8;
        true
    }
    pub fn clear(&mut self) {
        self.bytes.fill(0);
        self.len = 0;
    }
}
/// Pointer targets of the gate card.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Hit {
    Entry,
    Confirm,
    Reveal,
    Submit,
    /// Enrollment: "Use a PIN instead" / "Use a password instead".
    Switch,
    /// Sign-in with two kinds: the "Sign-in options" link.
    Options,
    /// Sign-in with two kinds: the option tile of a kind.
    Choice(Kind),
    /// A user tile at the bottom left (index in the account list).
    User(u8),
}
/// Keyboard focus. With one kind (and on enrollment) it never leaves the
/// secret box; with two kinds Tab also reaches "Sign-in options" and, while
/// they are shown, the option tiles.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Focus {
    Entry,
    Options,
    Choice(Kind),
}
/// Logical card geometry shared by painting and hit testing.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GateLayout {
    pub avatar: Rect,
    pub title_top: i32,
    pub subtitle_top: i32,
    pub entry: Rect,
    pub confirm: Rect,
    pub reveal: Rect,
    pub submit: Rect,
    pub notice_top: i32,
    /// Enrollment: the kind switch; sign-in with two kinds: "Sign-in options".
    pub switch: Rect,
    /// The sign-in option tiles below "Sign-in options": PIN, then password
    /// (the Windows order).
    pub choices: [Rect; 2],
    /// The other users at the bottom left (Windows lists every local
    /// account there when there is more than one), top to bottom.
    pub users: [Rect; 2],
}
/// A user tile at the bottom left: a 48 px avatar and the name.
pub const USER_TILE: (u32, u32) = (240, 64);
pub const FIELD_WIDTH: u32 = 296;
pub const FIELD_HEIGHT: u32 = 36;
/// Side of a sign-in option tile.
pub const CHOICE: u32 = 40;
/// The kinds of the option tiles, in `GateLayout::choices` order.
pub const CHOICES: [Kind; 2] = [Kind::Pin, Kind::Password];
impl GateLayout {
    pub fn new(width: u32, height: u32, page: Page) -> Self {
        let center = width as i32 / 2;
        let middle = height as i32 / 2;
        let enroll = page == Page::Enroll;
        let left = center - FIELD_WIDTH as i32 / 2;
        let entry_top = middle + if enroll { 52 } else { 44 };
        let entry = Rect::new(left, entry_top, FIELD_WIDTH, FIELD_HEIGHT).unwrap();
        let confirm = Rect::new(left, entry_top + 48, FIELD_WIDTH, FIELD_HEIGHT).unwrap();
        let last = if enroll { confirm } else { entry };
        let switch = Rect::new(center - 110, last.bottom() + 44, 220, 24).unwrap();
        let choice = |x: i32| Rect::new(x, switch.bottom() + 8, CHOICE, CHOICE).unwrap();
        Self {
            avatar: Rect::new(center - 96, middle - 228, 192, 192).unwrap(),
            title_top: middle - 12,
            subtitle_top: middle + 22,
            entry,
            confirm,
            reveal: Rect::new(entry.right() - 72, entry.y() + 2, 32, 32).unwrap(),
            submit: Rect::new(last.right() - 36, last.y(), 36, FIELD_HEIGHT).unwrap(),
            notice_top: last.bottom() + 14,
            switch,
            choices: [choice(center - CHOICE as i32 - 4), choice(center + 4)],
            users: [
                Rect::new(
                    24,
                    height as i32 - 24 - 2 * USER_TILE.1 as i32 - 4,
                    USER_TILE.0,
                    USER_TILE.1,
                )
                .unwrap(),
                Rect::new(
                    24,
                    height as i32 - 24 - USER_TILE.1 as i32,
                    USER_TILE.0,
                    USER_TILE.1,
                )
                .unwrap(),
            ],
        }
    }
    /// Sign-in with two kinds: "Sign-in options" and, while `open`, the
    /// option tiles (checked after `hit` found nothing).
    pub fn option_hit(&self, open: bool, x: i32, y: i32) -> Option<Hit> {
        if self.switch.contains(x, y) {
            return Some(Hit::Options);
        }
        if !open {
            return None;
        }
        self.choices
            .iter()
            .zip(CHOICES)
            .find(|(rect, _)| rect.contains(x, y))
            .map(|(_, kind)| Hit::Choice(kind))
    }
    /// A user tile at the bottom left (`count` accounts listed).
    pub fn user_hit(&self, count: usize, x: i32, y: i32) -> Option<Hit> {
        self.users
            .iter()
            .take(count)
            .position(|r| r.contains(x, y))
            .map(|i| Hit::User(i as u8))
    }
    pub fn hit(&self, page: Page, x: i32, y: i32) -> Option<Hit> {
        let enroll = page == Page::Enroll;
        if matches!(page, Page::Loading | Page::Unavailable) {
            return None;
        }
        if self.submit.contains(x, y) {
            Some(Hit::Submit)
        } else if self.reveal.contains(x, y) {
            Some(Hit::Reveal)
        } else if self.entry.contains(x, y) {
            Some(Hit::Entry)
        } else if enroll && self.confirm.contains(x, y) {
            Some(Hit::Confirm)
        } else if enroll && self.switch.contains(x, y) {
            Some(Hit::Switch)
        } else {
            None
        }
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Pending {
    State,
    Enroll,
    Verify,
}
/// The gate state machine. It never keeps a secret beyond the typing in
/// progress and wipes it on submit, failure, reset and drop.
pub struct Gate {
    purpose: Purpose,
    page: Page,
    /// The local accounts (two: the picker is shown) and the one signing in.
    accounts: Option<door_service_protocol::credential::Accounts>,
    selected: u32,
    kind: Kind,
    entry: Secret,
    confirm: Secret,
    confirm_focus: bool,
    reveal: bool,
    notice: Notice,
    failures: u32,
    retry_at: u64,
    persisted: bool,
    /// A password and a PIN are both enrolled ("Sign-in options" is shown).
    both: bool,
    /// The sign-in option tiles are shown.
    options: bool,
    focus: Focus,
    pending: Option<Pending>,
    press: Option<Hit>,
    translator: Translator,
    last_second: u64,
    /// The unavailable surface was reported; retries stay quiet until a
    /// surface is shown (the window server has a small serial budget).
    unavailable_reported: bool,
}
impl Drop for Gate {
    fn drop(&mut self) {
        self.entry.clear();
        self.confirm.clear();
    }
}
/// A copy of what the card shows; the secret appears only as dots unless the
/// reveal button is held.
#[derive(Clone, Copy, Debug)]
pub struct View {
    pub purpose: Purpose,
    pub page: Page,
    pub kind: Kind,
    pub entry: Secret,
    pub confirm: Secret,
    pub confirm_focus: bool,
    pub reveal: bool,
    pub notice: Notice,
    /// Whole seconds until another attempt (throttle notice).
    pub retry_seconds: u64,
    pub busy: bool,
    /// "Sign-in options" is offered (two kinds enrolled, sign-in page).
    pub both: bool,
    /// The option tiles are shown.
    pub options: bool,
    pub focus: Focus,
    /// Two accounts: the picker at the bottom left.
    pub users: Option<door_service_protocol::credential::Accounts>,
    /// The account signing in or unlocking.
    pub selected: u32,
}
struct Cursor<'a> {
    out: &'a mut [u8],
    at: usize,
}
impl core::fmt::Write for Cursor<'_> {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        let end = self.at + s.len();
        if end > self.out.len() {
            return Err(core::fmt::Error);
        }
        self.out[self.at..end].copy_from_slice(s.as_bytes());
        self.at = end;
        Ok(())
    }
}
/// Serial evidence; never contains secret material.
fn marker(text: core::fmt::Arguments<'_>) {
    let mut line = [0u8; 160];
    let mut cursor = Cursor {
        out: &mut line,
        at: 0,
    };
    if core::fmt::write(&mut cursor, text).is_ok() {
        let at = cursor.at;
        let _ = door_sys::write(&line[..at]);
    }
}
const fn kind_name(kind: Kind) -> &'static str {
    match kind {
        Kind::Pin => "pin",
        Kind::Password => "password",
    }
}
const fn purpose_name(purpose: Purpose) -> &'static str {
    match purpose {
        Purpose::SignIn => "signin",
        Purpose::Unlock => "unlock",
    }
}
const fn other(kind: Kind) -> Kind {
    kind.other()
}
impl Gate {
    pub fn new(purpose: Purpose) -> Self {
        Self {
            purpose,
            page: Page::Loading,
            accounts: None,
            selected: door_service_protocol::credential::ACCOUNT_UIDS[0],
            kind: Kind::Pin,
            entry: Secret::EMPTY,
            confirm: Secret::EMPTY,
            confirm_focus: false,
            reveal: false,
            notice: Notice::None,
            failures: 0,
            retry_at: 0,
            persisted: false,
            both: false,
            options: false,
            focus: Focus::Entry,
            pending: None,
            press: None,
            translator: Translator::new(),
            last_second: 0,
            unavailable_reported: false,
        }
    }
    pub fn purpose(&self) -> Purpose {
        self.purpose
    }
    pub fn page(&self) -> Page {
        self.page
    }
    pub fn busy(&self) -> bool {
        self.pending.is_some()
    }
    /// Restarts for `purpose` (a new lock, or after sign-in completed): the
    /// typed secret is wiped and the state queried again.
    pub fn reset(&mut self, purpose: Purpose, backend: &mut dyn Backend, now: u64) -> Action {
        let (failures, retry_at) = (self.failures, self.retry_at);
        let reported = self.unavailable_reported;
        let selected = self.selected;
        *self = Self::new(purpose);
        self.selected = selected;
        // The service stays authoritative; this only avoids a flash of the
        // countdown while the state reloads.
        self.failures = failures;
        self.retry_at = retry_at;
        self.unavailable_reported = reported;
        self.start(backend, now)
    }
    /// The account signing in (or unlocking).
    pub fn account(&self) -> u32 {
        self.selected
    }
    /// The accounts shown at the bottom left (sign-in with two accounts).
    pub fn users(&self) -> Option<door_service_protocol::credential::Accounts> {
        self.accounts
            .filter(|a| self.purpose == Purpose::SignIn && a.iter().count() > 1)
    }
    /// Unlock checks the signed-in user's credentials.
    pub fn select_account(&mut self, uid: u32) {
        self.selected = uid;
    }
    /// Queries the credential state (once after construction or reset).
    pub fn start(&mut self, backend: &mut dyn Backend, now: u64) -> Action {
        if self.pending.is_none() {
            self.accounts = backend.accounts();
            if self
                .accounts
                .is_some_and(|a| a.get(self.selected).is_none())
            {
                self.selected = door_service_protocol::credential::ACCOUNT_UIDS[0];
            }
            if !backend.select(self.selected) {
                self.unavailable();
                return Action::Redraw;
            }
            if backend.begin(Request::State, now) {
                self.pending = Some(Pending::State);
            } else {
                self.unavailable();
            }
        }
        Action::Redraw
    }
    pub fn view(&self, now: u64) -> View {
        View {
            purpose: self.purpose,
            page: self.page,
            kind: self.kind,
            entry: self.entry,
            confirm: self.confirm,
            confirm_focus: self.confirm_focus,
            reveal: self.reveal,
            notice: self.notice,
            retry_seconds: self.retry_at.saturating_sub(now).div_ceil(100),
            busy: self.pending.is_some(),
            both: self.offers_options(),
            options: self.options,
            focus: self.focus,
            users: self.users(),
            selected: self.selected,
        }
    }
    /// The sign-in page with a password and a PIN: "Sign-in options".
    fn offers_options(&self) -> bool {
        self.page == Page::SignIn && self.both
    }
    fn unavailable(&mut self) {
        self.pending = None;
        self.entry.clear();
        self.confirm.clear();
        self.options = false;
        self.focus = Focus::Entry;
        self.page = Page::Unavailable;
        if !self.unavailable_reported {
            self.unavailable_reported = true;
            marker(format_args!(
                "DOOR_P14 SIGNIN surface=unavailable purpose={}\n",
                purpose_name(self.purpose)
            ));
        }
    }
    fn surface_marker(&self) {
        marker(format_args!(
            "DOOR_P14 SIGNIN surface={} kind={} purpose={} persisted={}\n",
            if self.page == Page::Enroll {
                "enroll"
            } else {
                "signin"
            },
            kind_name(self.kind),
            purpose_name(self.purpose),
            u8::from(self.persisted)
        ));
    }
    fn show(&mut self, state: State, now: u64) {
        self.unavailable_reported = false;
        self.persisted = state.persisted;
        self.failures = state.failures;
        self.retry_at = now.saturating_add(state.retry_after);
        self.both = state.enrolled.is_some() && state.both;
        self.options = false;
        self.focus = Focus::Entry;
        self.page = match state.enrolled {
            // With both kinds this is the PIN, the Windows default.
            Some(kind) => {
                self.kind = kind;
                Page::SignIn
            }
            // Unlock never enrolls: a signed-in session already has a record.
            None if self.purpose == Purpose::Unlock => {
                self.unavailable();
                return;
            }
            None => Page::Enroll,
        };
        if self.retry_at > now {
            self.notice = Notice::Throttled;
        }
        self.surface_marker();
    }
    /// Polls an outstanding request; call every loop iteration.
    pub fn tick(&mut self, backend: &mut dyn Backend, now: u64) -> Action {
        let Some(pending) = self.pending else {
            // Repaint once per second while a throttle countdown runs.
            if self.notice == Notice::Throttled && now / 100 != self.last_second {
                self.last_second = now / 100;
                if now >= self.retry_at {
                    self.notice = Notice::None;
                }
                return Action::Redraw;
            }
            return Action::None;
        };
        let polled = backend.poll(now);
        if polled == Polled::Pending {
            return Action::None;
        }
        self.pending = None;
        match (pending, polled) {
            (Pending::State, Polled::State(state)) => {
                self.show(state, now);
                Action::Redraw
            }
            (Pending::Enroll, Polled::Enrolled(state)) if state.enrolled == Some(self.kind) => {
                self.persisted = state.persisted;
                marker(format_args!(
                    "DOOR_P14 SIGNIN enrolled kind={} persisted={}\n",
                    kind_name(self.kind),
                    u8::from(self.persisted)
                ));
                self.granted()
            }
            (Pending::Verify, Polled::Verdict(Verdict::Accepted)) => {
                self.failures = 0;
                self.retry_at = 0;
                self.granted()
            }
            (
                Pending::Verify,
                Polled::Verdict(Verdict::Rejected {
                    failures,
                    retry_after,
                }),
            ) => {
                self.failures = failures;
                self.retry_at = now.saturating_add(retry_after);
                self.notice = if retry_after > 0 {
                    Notice::Throttled
                } else {
                    Notice::Incorrect
                };
                marker(format_args!(
                    "DOOR_P14 SIGNIN rejected failures={failures} retry_ms={}\n",
                    retry_after.saturating_mul(10)
                ));
                Action::Redraw
            }
            (
                Pending::Verify,
                Polled::Verdict(Verdict::Throttled {
                    failures,
                    retry_after,
                }),
            ) => {
                self.failures = failures;
                self.retry_at = now.saturating_add(retry_after);
                self.notice = Notice::Throttled;
                marker(format_args!(
                    "DOOR_P14 SIGNIN throttled failures={failures} retry_ms={}\n",
                    retry_after.saturating_mul(10)
                ));
                Action::Redraw
            }
            _ => {
                self.unavailable();
                Action::Redraw
            }
        }
    }
    fn granted(&mut self) -> Action {
        self.entry.clear();
        self.confirm.clear();
        self.reveal = false;
        self.notice = Notice::None;
        marker(format_args!(
            "DOOR_P14 SIGNIN accepted purpose={}\n",
            purpose_name(self.purpose)
        ));
        Action::Granted
    }
    fn field(&mut self) -> &mut Secret {
        if self.confirm_focus {
            &mut self.confirm
        } else {
            &mut self.entry
        }
    }
    fn submit(&mut self, backend: &mut dyn Backend, now: u64) -> Action {
        if self.pending.is_some() || !matches!(self.page, Page::Enroll | Page::SignIn) {
            return Action::None;
        }
        if self.page == Page::Enroll {
            if !self.confirm_focus && self.confirm.is_empty() {
                // Enter in the first box moves on to the confirmation.
                self.confirm_focus = true;
                return Action::Redraw;
            }
            if credential::validate(self.kind, self.entry.as_bytes()).is_err() {
                self.notice = Notice::Policy;
                self.confirm.clear();
                self.confirm_focus = false;
                return Action::Redraw;
            }
            if self.entry != self.confirm {
                self.notice = Notice::Mismatch;
                self.entry.clear();
                self.confirm.clear();
                self.confirm_focus = false;
                return Action::Redraw;
            }
            let secret = self.entry;
            if !backend.begin(Request::Enroll(self.kind, secret.as_bytes()), now) {
                self.unavailable();
                return Action::Redraw;
            }
            self.pending = Some(Pending::Enroll);
            self.notice = Notice::Working;
            return Action::Redraw;
        }
        if self.entry.is_empty() {
            return Action::None;
        }
        if now < self.retry_at {
            // Refused here without contacting the service; the service
            // enforces the same deadline for any request that reaches it.
            self.notice = Notice::Throttled;
            self.entry.clear();
            marker(format_args!(
                "DOOR_P14 SIGNIN throttled failures={} retry_ms={} source=gate\n",
                self.failures,
                (self.retry_at - now).saturating_mul(10)
            ));
            return Action::Redraw;
        }
        let mut secret = self.entry;
        self.entry.clear();
        self.reveal = false;
        // Input the record's policy can never match (a password typed at a
        // PIN prompt) is still sent, framed as the other kind, so the service
        // counts it as a failure and the throttle applies uniformly.
        let kind = if credential::validate(self.kind, secret.as_bytes()).is_ok() {
            Some(self.kind)
        } else if credential::validate(other(self.kind), secret.as_bytes()).is_ok() {
            Some(other(self.kind))
        } else {
            None
        };
        let sent =
            kind.is_some_and(|kind| backend.begin(Request::Verify(kind, secret.as_bytes()), now));
        secret.clear();
        if !sent {
            // Too short for either policy: rejected locally, never counted.
            self.notice = Notice::Incorrect;
            return Action::Redraw;
        }
        self.pending = Some(Pending::Verify);
        self.notice = Notice::Working;
        Action::Redraw
    }
    /// Sign-in with two kinds: ask for `kind` from now on. The typed secret
    /// is wiped; a running throttle countdown stays (it is shared by both).
    fn choose(&mut self, kind: Kind) -> Action {
        if !self.offers_options() || self.pending.is_some() {
            return Action::None;
        }
        self.options = false;
        self.focus = Focus::Entry;
        if kind != self.kind {
            self.kind = kind;
            self.entry.clear();
            self.reveal = false;
            if self.notice != Notice::Throttled {
                self.notice = Notice::None;
            }
            self.surface_marker();
        }
        Action::Redraw
    }
    /// Enter or Space on the focused link or tile.
    fn activate(&mut self) -> Action {
        match self.focus {
            Focus::Entry => Action::None,
            Focus::Options => {
                self.options = !self.options;
                Action::Redraw
            }
            Focus::Choice(kind) => self.choose(kind),
        }
    }
    /// Tab order: box, "Sign-in options", then the tiles while shown;
    /// Shift+Tab walks it backwards.
    fn cycle(&mut self, backwards: bool) -> Action {
        let mut order = [Focus::Entry; 4];
        order[1] = Focus::Options;
        let mut len = 2;
        if self.options {
            for kind in CHOICES {
                order[len] = Focus::Choice(kind);
                len += 1;
            }
        }
        let at = order[..len]
            .iter()
            .position(|&f| f == self.focus)
            .unwrap_or(0);
        let next = (if backwards { at + len - 1 } else { at + 1 }) % len;
        self.focus = order[next];
        Action::Redraw
    }
    /// One key event (scan `code`, display `flags`). The gate consumes every
    /// key while it is shown; typing uses the US layout until sign-in.
    pub fn key(&mut self, backend: &mut dyn Backend, code: u32, flags: u32, now: u64) -> Action {
        let down = flags & door_abi::graphics::DOWN != 0;
        let translation = self.translator.translate(&layouts::US, code, flags);
        if !down || self.pending.is_some() {
            return Action::None;
        }
        // Keys that only exist while "Sign-in options" is offered.
        let on_option = self.offers_options() && self.focus != Focus::Entry;
        match code {
            0x1c | 0x11c | 0x39 if on_option => self.activate(),
            0x0f if self.offers_options() => self.cycle(flags & door_abi::graphics::SHIFT != 0),
            0x14b | 0x14d if on_option && matches!(self.focus, Focus::Choice(_)) => {
                let index = usize::from(code == 0x14d);
                self.focus = Focus::Choice(CHOICES[index]);
                Action::Redraw
            }
            0x1c | 0x11c => self.submit(backend, now),
            0x0e => {
                if !on_option && self.field().pop() {
                    Action::Redraw
                } else {
                    Action::None
                }
            }
            0x01 => {
                self.entry.clear();
                self.confirm.clear();
                self.confirm_focus = false;
                self.options = false;
                self.focus = Focus::Entry;
                self.notice = Notice::None;
                Action::Redraw
            }
            0x0f if self.page == Page::Enroll => {
                self.confirm_focus = !self.confirm_focus;
                Action::Redraw
            }
            _ => {
                if !matches!(self.page, Page::Enroll | Page::SignIn) || on_option {
                    return Action::None;
                }
                let kind = self.kind;
                let mut changed = false;
                for &ch in translation.text.chars() {
                    changed |= self.field().push(kind, ch);
                }
                if changed
                    && matches!(
                        self.notice,
                        Notice::Incorrect | Notice::Mismatch | Notice::Policy
                    )
                {
                    self.notice = Notice::None;
                }
                if changed {
                    Action::Redraw
                } else {
                    Action::None
                }
            }
        }
    }
    /// Primary-button press/release at logical `point` on a `size` logical
    /// output. The reveal button shows the secret only while held.
    pub fn pointer(
        &mut self,
        backend: &mut dyn Backend,
        point: (i32, i32),
        size: (u32, u32),
        down: bool,
        now: u64,
    ) -> Action {
        let layout = GateLayout::new(size.0, size.1, self.page);
        let users = self.users().map_or(0, |a| a.iter().count());
        let hit = layout
            .hit(self.page, point.0, point.1)
            .or_else(|| {
                self.offers_options()
                    .then(|| layout.option_hit(self.options, point.0, point.1))
                    .flatten()
            })
            .or_else(|| layout.user_hit(users, point.0, point.1));
        if down {
            self.press = hit;
            if hit == Some(Hit::Reveal) && !self.reveal {
                self.reveal = true;
                return Action::Redraw;
            }
            return Action::None;
        }
        let pressed = self.press.take();
        let was_revealed = core::mem::take(&mut self.reveal);
        let action = match (pressed, hit) {
            (Some(a), Some(b)) if a == b => match a {
                Hit::Entry => {
                    self.confirm_focus = false;
                    self.focus = Focus::Entry;
                    Action::Redraw
                }
                Hit::Options if self.pending.is_none() => {
                    self.options = !self.options;
                    if !self.options && matches!(self.focus, Focus::Choice(_)) {
                        self.focus = Focus::Options;
                    }
                    Action::Redraw
                }
                Hit::Choice(kind) => self.choose(kind),
                Hit::User(index) if self.pending.is_none() => {
                    let uid = self
                        .users()
                        .and_then(|a| a.iter().nth(usize::from(index)).map(|a| a.uid));
                    match uid {
                        Some(uid) if uid != self.selected => {
                            self.selected = uid;
                            marker(format_args!("DOOR_P16 SIGNIN account={uid}\n"));
                            self.reset(self.purpose, backend, now)
                        }
                        _ => Action::None,
                    }
                }
                Hit::Confirm => {
                    self.confirm_focus = true;
                    Action::Redraw
                }
                Hit::Submit => self.submit(backend, now),
                Hit::Switch if self.pending.is_none() => {
                    self.kind = other(self.kind);
                    self.entry.clear();
                    self.confirm.clear();
                    self.confirm_focus = false;
                    self.notice = Notice::None;
                    self.surface_marker();
                    Action::Redraw
                }
                _ => Action::None,
            },
            _ => Action::None,
        };
        if was_revealed && action == Action::None {
            Action::Redraw
        } else {
            action
        }
    }
}

const TEXT: [u8; 4] = [255; 4];
const DIM: [u8; 4] = [200, 200, 200, 255];
const FIELD: [u8; 4] = [20, 20, 20, 150];
const SUBMIT: [u8; 4] = [46, 46, 46, 46];

fn label(
    canvas: &mut Canvas<'_>,
    fonts: &FontAtlas,
    value: &str,
    style: TextStyle,
    (x, top): (i32, i32),
    centered: bool,
    color: [u8; 4],
) {
    let scale = canvas.scale();
    let Some(clip) = canvas.clip_rect() else {
        return;
    };
    let x = if centered {
        let width = fonts.measure(value, style).map_or(0, |m| m.advance_width);
        x - width as i32 / 2
    } else {
        x
    };
    let (pw, ph, stride) = canvas.physical_dimensions();
    if let Ok(mut target) = Target::new(canvas.pixels_mut(), pw, ph, stride) {
        let _ = fonts.draw_physical(
            &mut target,
            Clip::new(clip.x(), clip.y(), clip.width(), clip.height()),
            TextRun {
                text: value,
                x: scale.px(x),
                top: scale.px(top),
                style,
                color,
            },
        );
    }
}
fn eye_mask(factor: u32) -> &'static [u8] {
    match factor {
        3 => include_bytes!("../../../assets/icons/windows-shell/setting-eye.60.mask"),
        2 => include_bytes!("../../../assets/icons/windows-shell/setting-eye.40.mask"),
        _ => include_bytes!("../../../assets/icons/windows-shell/setting-eye.20.mask"),
    }
}
fn forward_mask(factor: u32) -> &'static [u8] {
    match factor {
        3 => include_bytes!("../../../assets/icons/windows-shell/app-forward.60.mask"),
        2 => include_bytes!("../../../assets/icons/windows-shell/app-forward.40.mask"),
        _ => include_bytes!("../../../assets/icons/windows-shell/app-forward.20.mask"),
    }
}
fn secret_box(
    canvas: &mut Canvas<'_>,
    fonts: &FontAtlas,
    rect: Rect,
    secret: &Secret,
    placeholder: &str,
    focused: bool,
    reveal: bool,
) {
    let _ = canvas.fill_rounded_rect(rect, 4, FIELD);
    if focused {
        let line = Rect::new(rect.x(), rect.bottom() - 2, rect.width(), 2).unwrap();
        let _ = canvas.fill_rect(line, TEXT);
    }
    let left = rect.x() + 12;
    let top = rect.y() + 9;
    if secret.is_empty() {
        label(
            canvas,
            fonts,
            placeholder,
            TextStyle::Regular14,
            (left, top),
            false,
            DIM,
        );
    } else if reveal {
        let value = core::str::from_utf8(secret.as_bytes()).unwrap_or("");
        label(
            canvas,
            fonts,
            value,
            TextStyle::Regular14,
            (left, top),
            false,
            TEXT,
        );
    } else {
        // One dot per character, as the Windows password box shows.
        for index in 0..secret.chars().min(20) {
            let dot = Rect::new(left + index as i32 * 11, rect.y() + 14, 7, 7).unwrap();
            let _ = canvas.fill_rounded_rect(dot, 4, TEXT);
        }
    }
}
/// Paints the gate card over the lock or sign-in backdrop already in `canvas`.
pub fn draw_gate(canvas: &mut Canvas<'_>, fonts: &FontAtlas, view: &View) {
    let (width, height, _) = canvas.dimensions();
    let scale = canvas.scale();
    let factor = scale.asset_factor();
    let center = width as i32 / 2;
    let layout = GateLayout::new(width, height, view.page);
    let _ = canvas.fill_rect(Rect::new(0, 0, width, height).unwrap(), [30, 14, 4, 130]);
    let _ = canvas.fill_rounded_rect(layout.avatar, 96, [18, 18, 18, 18]);
    let contact: &[u8] = match factor {
        3 => include_bytes!("../../../assets/icons/windows-shell/contact.288.mask"),
        2 => include_bytes!("../../../assets/icons/windows-shell/contact.192.mask"),
        _ => include_bytes!("../../../assets/icons/windows-shell/contact.96.mask"),
    };
    crate::tray::glyph(
        canvas,
        center - 48,
        layout.avatar.y() + 48,
        96,
        contact,
        TEXT,
    );
    let enroll = view.page == Page::Enroll;
    let name = view.users.and_then(|a| {
        a.get(view.selected)
            .map(|a| alloc::string::String::from(a.name()))
    });
    let title = match (enroll, view.kind) {
        (true, Kind::Pin) => "Create a PIN",
        (true, Kind::Password) => "Create a password",
        (false, _) => name.as_deref().unwrap_or("Local account"),
    };
    // Every local account at the bottom left, the selected one highlighted.
    if let Some(users) = view.users {
        for (account, rect) in users.iter().zip(layout.users) {
            if account.uid == view.selected {
                let _ = canvas.fill_rounded_rect(rect, 4, [255, 255, 255, 40]);
            }
            let avatar = Rect::new(rect.x() + 8, rect.y() + 8, 48, 48).unwrap();
            let _ = canvas.fill_rounded_rect(avatar, 24, [18, 18, 18, 18]);
            let glyph: &[u8] = match factor {
                3 => include_bytes!("../../../assets/icons/windows-shell/contact.72.mask"),
                2 => include_bytes!("../../../assets/icons/windows-shell/contact.48.mask"),
                _ => include_bytes!("../../../assets/icons/windows-shell/contact.24.mask"),
            };
            crate::tray::glyph(canvas, avatar.x() + 12, avatar.y() + 12, 24, glyph, TEXT);
            label(
                canvas,
                fonts,
                account.name(),
                TextStyle::Regular14,
                (avatar.right() + 12, rect.y() + 22),
                false,
                TEXT,
            );
        }
    }
    label(
        canvas,
        fonts,
        title,
        TextStyle::Semibold28,
        (center, layout.title_top),
        true,
        TEXT,
    );
    if enroll {
        let subtitle = "Make it something you'll remember.";
        label(
            canvas,
            fonts,
            subtitle,
            TextStyle::Regular14,
            (center, layout.subtitle_top),
            true,
            TEXT,
        );
    }
    let status = match view.page {
        Page::Loading => Some("Please wait"),
        Page::Unavailable => Some("Sign-in isn't available right now."),
        Page::Enroll | Page::SignIn => None,
    };
    if let Some(status) = status {
        label(
            canvas,
            fonts,
            status,
            TextStyle::Regular14,
            (center, layout.entry.y() + 9),
            true,
            TEXT,
        );
        return;
    }
    let (first, second) = match (enroll, view.kind) {
        (true, Kind::Pin) => ("New PIN", "Confirm PIN"),
        (true, Kind::Password) => ("New password", "Confirm password"),
        (false, Kind::Pin) => ("PIN", ""),
        (false, Kind::Password) => ("Password", ""),
    };
    let entry_focus = !view.confirm_focus;
    secret_box(
        canvas,
        fonts,
        layout.entry,
        &view.entry,
        first,
        entry_focus,
        view.reveal,
    );
    if enroll {
        let confirm = &view.confirm;
        secret_box(
            canvas,
            fonts,
            layout.confirm,
            confirm,
            second,
            view.confirm_focus,
            view.reveal,
        );
    }
    let (rx, ry) = (layout.reveal.x() + 6, layout.reveal.y() + 6);
    crate::tray::glyph(canvas, rx, ry, 20, eye_mask(factor), DIM);
    let _ = canvas.fill_rounded_rect(layout.submit, 4, SUBMIT);
    let (sx, sy) = (layout.submit.x() + 8, layout.submit.y() + 8);
    crate::tray::glyph(canvas, sx, sy, 20, forward_mask(factor), TEXT);
    let mut countdown = [0u8; 96];
    let notice: &str = match (view.notice, view.kind) {
        (Notice::None, _) => "",
        (Notice::Working, _) if enroll => "Setting up...",
        (Notice::Working, _) => "Welcome",
        (Notice::Incorrect, Kind::Pin) => "The PIN is incorrect. Try again.",
        (Notice::Incorrect, Kind::Password) => "The password is incorrect. Try again.",
        (Notice::Mismatch, Kind::Pin) => "The PINs don't match. Try again.",
        (Notice::Mismatch, Kind::Password) => "The passwords don't match. Try again.",
        (Notice::Policy, Kind::Pin) => "Use at least 4 digits.",
        (Notice::Policy, Kind::Password) => "Use at least 4 characters.",
        (Notice::Throttled, _) => {
            let mut cursor = Cursor {
                out: &mut countdown,
                at: 0,
            };
            let seconds = view.retry_seconds.max(1);
            let _ = core::fmt::write(
                &mut cursor,
                format_args!("Too many attempts. Try again in {seconds} seconds."),
            );
            let at = cursor.at;
            core::str::from_utf8(&countdown[..at]).unwrap_or("")
        }
    };
    if !notice.is_empty() {
        label(
            canvas,
            fonts,
            notice,
            TextStyle::Regular14,
            (center, layout.notice_top),
            true,
            TEXT,
        );
    }
    if enroll {
        let link = match view.kind {
            Kind::Pin => "Use a password instead",
            Kind::Password => "Use a PIN instead",
        };
        label(
            canvas,
            fonts,
            link,
            TextStyle::Regular14,
            (center, layout.switch.y() + 3),
            true,
            TEXT,
        );
    }
    if view.both && view.page == Page::SignIn {
        sign_in_options(canvas, fonts, &layout, view);
    }
}

/// A 2-logical-pixel keyboard focus rectangle just outside `rect`.
fn focus_ring(canvas: &mut Canvas<'_>, rect: Rect) {
    let (x, y) = (rect.x() - 3, rect.y() - 3);
    let (w, h) = (rect.width() + 6, rect.height() + 6);
    for edge in [
        Rect::new(x, y, w, 2),
        Rect::new(x, y + h as i32 - 2, w, 2),
        Rect::new(x, y, 2, h),
        Rect::new(x + w as i32 - 2, y, 2, h),
    ]
    .into_iter()
    .flatten()
    {
        let _ = canvas.fill_rect(edge, TEXT);
    }
}
fn key_mask(factor: u32) -> &'static [u8] {
    match factor {
        3 => include_bytes!("../../../assets/icons/windows-shell/setting-key.60.mask"),
        2 => include_bytes!("../../../assets/icons/windows-shell/setting-key.40.mask"),
        _ => include_bytes!("../../../assets/icons/windows-shell/setting-key.20.mask"),
    }
}
/// The PIN option glyph: a dial pad (three rows of three keys and a zero) in
/// a 20-logical-pixel square at (`x`, `y`).
fn dial_pad(canvas: &mut Canvas<'_>, x: i32, y: i32, color: [u8; 4]) {
    let mut key = |column: i32, row: i32| {
        if let Ok(dot) = Rect::new(x + 2 + column * 6, y + row * 5, 4, 4) {
            let _ = canvas.fill_rounded_rect(dot, 2, color);
        }
    };
    for row in 0..3 {
        for column in 0..3 {
            key(column, row);
        }
    }
    key(1, 3);
}
/// "Sign-in options" and, while open, one tile per kind (the current kind
/// highlighted), as the Windows lock screen offers them.
fn sign_in_options(canvas: &mut Canvas<'_>, fonts: &FontAtlas, layout: &GateLayout, view: &View) {
    let factor = canvas.scale().asset_factor();
    let center = layout.switch.x() + layout.switch.width() as i32 / 2;
    let text = "Sign-in options";
    label(
        canvas,
        fonts,
        text,
        TextStyle::Regular14,
        (center, layout.switch.y() + 3),
        true,
        TEXT,
    );
    if view.focus == Focus::Options {
        let width = fonts
            .measure(text, TextStyle::Regular14)
            .map_or(layout.switch.width(), |m| m.advance_width);
        let width = width.min(layout.switch.width());
        if let Ok(ring) = Rect::new(
            center - width as i32 / 2,
            layout.switch.y() + 2,
            width,
            layout.switch.height() - 4,
        ) {
            focus_ring(canvas, ring);
        }
    }
    if !view.options {
        return;
    }
    for (rect, kind) in layout.choices.into_iter().zip(CHOICES) {
        let fill = if kind == view.kind {
            [64, 64, 64, 64]
        } else {
            [24, 24, 24, 24]
        };
        let _ = canvas.fill_rounded_rect(rect, 4, fill);
        let (gx, gy) = (rect.x() + 10, rect.y() + 10);
        match kind {
            Kind::Pin => dial_pad(canvas, gx, gy, TEXT),
            Kind::Password => crate::tray::glyph(canvas, gx, gy, 20, key_mask(factor), TEXT),
        }
        if view.focus == Focus::Choice(kind) {
            focus_ring(canvas, rect);
        }
    }
}

#[cfg(test)]
mod tests {
    extern crate alloc;
    use super::*;
    #[test]
    fn pending_button_region_matches_full_login_without_repainting_avatar_or_name() {
        for factor in 1..=3 {
            let scale = door_graphics_core::Scale::from_factor(factor).unwrap();
            let (w, h) = (640 * factor, 480 * factor);
            let fonts = FontAtlas::windows_reference_scaled(factor).unwrap();
            let base = alloc::vec![170; (w * h * 4) as usize];
            let mut full = base.clone();
            let mut old = base.clone();
            for (pixels, pending) in [(&mut full, true), (&mut old, false)] {
                let mut canvas = Canvas::new(pixels, w, h, w as usize * 4).unwrap();
                canvas.set_scale(scale).unwrap();
                draw(&mut canvas, &fonts, pending);
            }
            let region = crate::taskbar::physical(Layout::new(640, 480).button, scale);
            let mut canvas = Canvas::new(&mut old, w, h, w as usize * 4).unwrap();
            canvas.set_clip(region);
            canvas.restore_bgra(&base).unwrap();
            canvas.set_scale(scale).unwrap();
            draw(&mut canvas, &fonts, true);
            assert_eq!(old, full);
        }
    }
    #[test]
    fn sign_in_metrics_keep_avatar_name_and_trusted_button_separate() {
        for (w, h) in [(1024, 768), (1280, 720), (1920, 1080), (2560, 1600)] {
            let l = Layout::new(w, h);
            assert_eq!(l.avatar.x() * 2 + l.avatar.width() as i32, w as i32);
            assert_eq!(l.button.x() * 2 + l.button.width() as i32, w as i32);
            assert!(l.avatar.bottom() + 20 <= l.name_top);
            assert!(l.name_top + 32 <= l.button.y());
            assert!(l.button.contains(l.button.x(), l.button.y()));
            assert!(!l.button.contains(l.button.right(), l.button.y()));
            assert!(l.button.bottom() < h as i32);
        }
    }

    struct Fake {
        enrolled: Option<Kind>,
        secret: alloc::vec::Vec<u8>,
        /// Two kinds: `secret` is the password and this the PIN.
        pin: Option<alloc::vec::Vec<u8>>,
        failures: u32,
        retry_at: u64,
        reply: Option<Polled>,
        sent: alloc::vec::Vec<alloc::vec::Vec<u8>>,
        kinds: alloc::vec::Vec<Kind>,
        offline: bool,
        accounts: Option<door_service_protocol::credential::Accounts>,
        selected: alloc::vec::Vec<u32>,
    }
    impl Fake {
        fn new(enrolled: Option<(Kind, &[u8])>) -> Self {
            Self {
                enrolled: enrolled.map(|e| e.0),
                secret: enrolled.map_or(alloc::vec::Vec::new(), |e| e.1.to_vec()),
                pin: None,
                failures: 0,
                retry_at: 0,
                reply: None,
                sent: alloc::vec::Vec::new(),
                kinds: alloc::vec::Vec::new(),
                offline: false,
                accounts: None,
                selected: alloc::vec::Vec::new(),
            }
        }
        fn both(password: &[u8], pin: &[u8]) -> Self {
            let mut fake = Self::new(Some((Kind::Password, password)));
            fake.pin = Some(pin.to_vec());
            fake
        }
        fn state(&self, now: u64) -> State {
            State {
                enrolled: if self.pin.is_some() {
                    Some(Kind::Pin)
                } else {
                    self.enrolled
                },
                both: self.pin.is_some(),
                persisted: true,
                failures: self.failures,
                retry_after: self.retry_at.saturating_sub(now),
            }
        }
    }
    impl Backend for Fake {
        fn accounts(&mut self) -> Option<door_service_protocol::credential::Accounts> {
            self.accounts
        }
        fn select(&mut self, uid: u32) -> bool {
            self.selected.push(uid);
            true
        }
        fn begin(&mut self, request: Request<'_>, now: u64) -> bool {
            if self.offline || self.reply.is_some() {
                return false;
            }
            self.reply = Some(match request {
                Request::State => Polled::State(self.state(now)),
                Request::Enroll(kind, secret) => {
                    self.sent.push(secret.to_vec());
                    self.enrolled = Some(kind);
                    self.secret = secret.to_vec();
                    Polled::Enrolled(self.state(now))
                }
                Request::Verify(kind, secret) => {
                    self.sent.push(secret.to_vec());
                    self.kinds.push(kind);
                    // With two kinds the framed kind selects the record.
                    let expected = match &self.pin {
                        Some(pin) if kind == Kind::Pin => pin.as_slice(),
                        _ => self.secret.as_slice(),
                    };
                    if now < self.retry_at {
                        Polled::Verdict(Verdict::Throttled {
                            failures: self.failures,
                            retry_after: self.retry_at - now,
                        })
                    } else if secret == expected {
                        self.failures = 0;
                        Polled::Verdict(Verdict::Accepted)
                    } else {
                        self.failures += 1;
                        let retry_after = credential::throttle_delay(self.failures);
                        self.retry_at = now + retry_after;
                        Polled::Verdict(Verdict::Rejected {
                            failures: self.failures,
                            retry_after,
                        })
                    }
                }
            });
            true
        }
        fn poll(&mut self, _now: u64) -> Polled {
            self.reply.take().unwrap_or(Polled::Failed)
        }
    }
    const DOWN: u32 = door_abi::graphics::DOWN;
    fn type_text(gate: &mut Gate, fake: &mut Fake, text: &str, now: u64) {
        for ch in text.chars() {
            let code = match ch {
                '1'..='9' => 1 + (ch as u32 - '0' as u32),
                '0' => 11,
                'a' => 0x1e,
                'b' => 0x30,
                'c' => 0x2e,
                'd' => 0x20,
                'o' => 0x18,
                'r' => 0x13,
                _ => panic!("{ch}"),
            };
            gate.key(fake, code, DOWN, now);
            gate.key(fake, code, 0, now);
        }
    }
    fn enter(gate: &mut Gate, fake: &mut Fake, now: u64) -> Action {
        let action = gate.key(fake, 0x1c, DOWN, now);
        gate.key(fake, 0x1c, 0, now);
        if gate.busy() {
            return gate.tick(fake, now);
        }
        action
    }

    #[test]
    fn two_accounts_show_the_picker_and_sign_in_the_chosen_one() {
        use door_service_protocol::credential::{Account, Accounts};
        let mut fake = Fake::new(Some((Kind::Password, b"owner-pass")));
        let mut accounts = Accounts::default();
        accounts.list[0] = Account::new(1001, "Owner", true).ok();
        accounts.list[1] = Account::new(1002, "Guest Two", false).ok();
        fake.accounts = Some(accounts);
        let mut gate = Gate::new(Purpose::SignIn);
        gate.start(&mut fake, 0);
        gate.tick(&mut fake, 0);
        assert_eq!(gate.account(), 1001);
        assert_eq!(fake.selected, [1001]);
        let view = gate.view(0);
        assert_eq!(view.users, Some(accounts));
        // The second tile at the bottom left picks the other account.
        let layout = GateLayout::new(1280, 720, gate.page());
        let tile = layout.users[1];
        let point = (tile.x() + 20, tile.y() + 20);
        gate.pointer(&mut fake, point, (1280, 720), true, 1);
        gate.pointer(&mut fake, point, (1280, 720), false, 1);
        gate.tick(&mut fake, 1);
        assert_eq!(gate.account(), 1002);
        assert_eq!(fake.selected, [1001, 1002]);
        // Unlock never shows the picker.
        gate.reset(Purpose::Unlock, &mut fake, 2);
        assert_eq!(gate.view(2).users, None);
        assert_eq!(gate.account(), 1002);
        // One account: no picker either.
        let mut single = Fake::new(Some((Kind::Password, b"owner-pass")));
        let mut gate = Gate::new(Purpose::SignIn);
        gate.start(&mut single, 0);
        assert_eq!(gate.view(0).users, None);
    }
    #[test]
    fn first_run_enrolls_a_confirmed_pin_then_signs_in() {
        let mut fake = Fake::new(None);
        let mut gate = Gate::new(Purpose::SignIn);
        gate.start(&mut fake, 0);
        gate.tick(&mut fake, 0);
        assert_eq!(gate.page(), Page::Enroll);
        // Letters never enter a PIN box.
        type_text(&mut gate, &mut fake, "12ab34", 1);
        assert_eq!(gate.view(1).entry.as_bytes(), b"1234");
        assert_eq!(enter(&mut gate, &mut fake, 1), Action::Redraw);
        assert!(gate.view(1).confirm_focus);
        type_text(&mut gate, &mut fake, "1235", 1);
        assert_eq!(enter(&mut gate, &mut fake, 1), Action::Redraw);
        assert_eq!(gate.view(1).notice, Notice::Mismatch);
        assert!(gate.view(1).entry.is_empty() && fake.sent.is_empty());
        type_text(&mut gate, &mut fake, "2468", 2);
        enter(&mut gate, &mut fake, 2);
        type_text(&mut gate, &mut fake, "2468", 2);
        assert_eq!(enter(&mut gate, &mut fake, 2), Action::Granted);
        assert_eq!(fake.sent, [b"2468".to_vec()]);
        assert!(gate.view(2).entry.is_empty() && gate.view(2).confirm.is_empty());
        // Unlock with a wrong then the right PIN.
        gate.reset(Purpose::Unlock, &mut fake, 3);
        gate.tick(&mut fake, 3);
        assert_eq!(gate.page(), Page::SignIn);
        type_text(&mut gate, &mut fake, "1111", 3);
        assert_eq!(enter(&mut gate, &mut fake, 3), Action::Redraw);
        assert_eq!(gate.view(3).notice, Notice::Incorrect);
        type_text(&mut gate, &mut fake, "2468", 4);
        assert_eq!(enter(&mut gate, &mut fake, 4), Action::Granted);
    }

    #[test]
    fn throttling_blocks_attempts_and_counts_down() {
        let mut fake = Fake::new(Some((Kind::Password, b"door")));
        let mut gate = Gate::new(Purpose::Unlock);
        gate.start(&mut fake, 0);
        gate.tick(&mut fake, 0);
        assert_eq!(gate.view(0).kind, Kind::Password);
        for _ in 0..credential::FREE_ATTEMPTS {
            type_text(&mut gate, &mut fake, "abcd", 10);
            enter(&mut gate, &mut fake, 10);
        }
        let view = gate.view(10);
        assert_eq!(view.notice, Notice::Throttled);
        assert_eq!(view.retry_seconds, credential::FIRST_DELAY_TICKS / 100);
        let sent = fake.sent.len();
        // The right secret during the throttle is not even sent.
        type_text(&mut gate, &mut fake, "door", 20);
        assert_eq!(enter(&mut gate, &mut fake, 20), Action::Redraw);
        assert_eq!(fake.sent.len(), sent);
        let later = 10 + credential::FIRST_DELAY_TICKS;
        assert_eq!(gate.tick(&mut fake, later), Action::Redraw);
        type_text(&mut gate, &mut fake, "door", later);
        assert_eq!(enter(&mut gate, &mut fake, later), Action::Granted);
    }

    #[test]
    fn reveal_is_held_and_unavailable_fails_closed() {
        let mut fake = Fake::new(Some((Kind::Pin, b"2468")));
        let mut gate = Gate::new(Purpose::SignIn);
        gate.start(&mut fake, 0);
        gate.tick(&mut fake, 0);
        let layout = GateLayout::new(1280, 720, Page::SignIn);
        let eye = (layout.reveal.x() + 4, layout.reveal.y() + 4);
        assert_eq!(
            gate.pointer(&mut fake, eye, (1280, 720), true, 0),
            Action::Redraw
        );
        assert!(gate.view(0).reveal);
        assert_eq!(
            gate.pointer(&mut fake, eye, (1280, 720), false, 0),
            Action::Redraw
        );
        assert!(!gate.view(0).reveal);
        // A dead service: no passwordless fallback, typing is ignored.
        let mut dead = Fake::new(None);
        dead.offline = true;
        let mut gate = Gate::new(Purpose::SignIn);
        gate.start(&mut dead, 0);
        assert_eq!(gate.page(), Page::Unavailable);
        type_text(&mut gate, &mut dead, "1234", 0);
        assert_eq!(enter(&mut gate, &mut dead, 0), Action::None);
        // Unlock never offers enrollment.
        let mut empty = Fake::new(None);
        let mut gate = Gate::new(Purpose::Unlock);
        gate.start(&mut empty, 0);
        gate.tick(&mut empty, 0);
        assert_eq!(gate.page(), Page::Unavailable);
    }

    #[test]
    fn gate_card_paints_at_every_density_without_secret_text() {
        for factor in 1..=3 {
            let scale = door_graphics_core::Scale::from_factor(factor).unwrap();
            let (w, h) = (640 * factor, 480 * factor);
            let fonts = FontAtlas::windows_reference_scaled(factor).unwrap();
            for page in [Page::Loading, Page::Enroll, Page::SignIn, Page::Unavailable] {
                let mut pixels = alloc::vec![90u8; (w * h * 4) as usize];
                let mut canvas = Canvas::new(&mut pixels, w, h, w as usize * 4).unwrap();
                canvas.set_scale(scale).unwrap();
                let mut entry = Secret::EMPTY;
                for ch in "2468".chars() {
                    entry.push(Kind::Pin, ch);
                }
                let view = View {
                    purpose: Purpose::SignIn,
                    page,
                    kind: Kind::Pin,
                    entry,
                    confirm: Secret::EMPTY,
                    confirm_focus: false,
                    reveal: false,
                    notice: Notice::Throttled,
                    retry_seconds: 30,
                    busy: false,
                    both: false,
                    options: false,
                    focus: Focus::Entry,
                    users: None,
                    selected: 1001,
                };
                draw_gate(&mut canvas, &fonts, &view);
                assert!(pixels.iter().any(|&b| b != 90));
            }
        }
        let l = GateLayout::new(1280, 720, Page::Enroll);
        assert!(l.entry.bottom() < l.confirm.y() && l.confirm.bottom() < l.notice_top);
        assert!(l.notice_top + 20 <= l.switch.y());
        assert_eq!(
            l.hit(Page::Enroll, l.switch.x() + 1, l.switch.y() + 1),
            Some(Hit::Switch)
        );
        assert_eq!(
            l.hit(Page::SignIn, l.switch.x() + 1, l.switch.y() + 1),
            None
        );
        assert_eq!(alloc::format!("{:?}", Secret::EMPTY), "Secret(0 chars)");
    }

    fn press(gate: &mut Gate, fake: &mut Fake, code: u32, flags: u32, now: u64) -> Action {
        let action = gate.key(fake, code, DOWN | flags, now);
        gate.key(fake, code, flags, now);
        action
    }
    fn click(gate: &mut Gate, fake: &mut Fake, point: (i32, i32), now: u64) -> Action {
        gate.pointer(fake, point, (1280, 720), true, now);
        gate.pointer(fake, point, (1280, 720), false, now)
    }
    fn inside(rect: Rect) -> (i32, i32) {
        (rect.x() + 2, rect.y() + 2)
    }
    const TAB: u32 = 0x0f;
    const SPACE: u32 = 0x39;
    const ESC: u32 = 0x01;
    const RIGHT: u32 = 0x14d;
    const LEFT: u32 = 0x14b;
    const SHIFT: u32 = door_abi::graphics::SHIFT;

    #[test]
    fn one_kind_offers_no_sign_in_options_and_keeps_its_keys() {
        for (kind, secret) in [(Kind::Pin, &b"2468"[..]), (Kind::Password, b"door")] {
            let mut fake = Fake::new(Some((kind, secret)));
            let mut gate = Gate::new(Purpose::SignIn);
            gate.start(&mut fake, 0);
            gate.tick(&mut fake, 0);
            let view = gate.view(0);
            assert_eq!(
                (view.page, view.kind, view.both),
                (Page::SignIn, kind, false)
            );
            // Tab, Space (in a PIN box), the arrows and the options area do
            // nothing; focus never leaves the box.
            for code in [TAB, RIGHT, LEFT] {
                assert_eq!(press(&mut gate, &mut fake, code, 0, 0), Action::None);
            }
            assert_eq!(press(&mut gate, &mut fake, TAB, SHIFT, 0), Action::None);
            if kind == Kind::Pin {
                assert_eq!(press(&mut gate, &mut fake, SPACE, 0, 0), Action::None);
            }
            let layout = GateLayout::new(1280, 720, Page::SignIn);
            assert_eq!(
                click(&mut gate, &mut fake, inside(layout.switch), 0),
                Action::None
            );
            assert_eq!(
                click(&mut gate, &mut fake, inside(layout.choices[1]), 0),
                Action::None
            );
            let view = gate.view(0);
            assert_eq!(
                (view.focus, view.options, view.kind),
                (Focus::Entry, false, kind)
            );
            let text = core::str::from_utf8(secret).unwrap();
            type_text(&mut gate, &mut fake, text, 1);
            assert_eq!(enter(&mut gate, &mut fake, 1), Action::Granted);
            assert_eq!(fake.kinds, [kind]);
        }
    }

    #[test]
    fn two_kinds_sign_in_with_the_pin_and_switch_to_the_password_by_pointer() {
        let mut fake = Fake::both(b"door", b"2468");
        let mut gate = Gate::new(Purpose::SignIn);
        gate.start(&mut fake, 0);
        gate.tick(&mut fake, 0);
        let view = gate.view(0);
        // The PIN is the default, as on Windows.
        assert_eq!(
            (view.page, view.kind, view.both, view.options),
            (Page::SignIn, Kind::Pin, true, false)
        );
        type_text(&mut gate, &mut fake, "2468", 1);
        assert_eq!(enter(&mut gate, &mut fake, 1), Action::Granted);
        // Win+L: unlock asks for the PIN again, with the same options.
        gate.reset(Purpose::Unlock, &mut fake, 2);
        gate.tick(&mut fake, 2);
        assert_eq!((gate.view(2).kind, gate.view(2).both), (Kind::Pin, true));
        let layout = GateLayout::new(1280, 720, Page::SignIn);
        // The tiles exist only once "Sign-in options" opened them.
        assert_eq!(
            click(&mut gate, &mut fake, inside(layout.choices[1]), 2),
            Action::None
        );
        type_text(&mut gate, &mut fake, "13", 2);
        assert_eq!(
            click(&mut gate, &mut fake, inside(layout.switch), 2),
            Action::Redraw
        );
        assert!(gate.view(2).options);
        assert_eq!(
            click(&mut gate, &mut fake, inside(layout.choices[1]), 2),
            Action::Redraw
        );
        let view = gate.view(2);
        assert_eq!(
            (view.kind, view.options, view.focus),
            (Kind::Password, false, Focus::Entry)
        );
        assert!(view.entry.is_empty(), "switching wipes the typed PIN");
        // A wrong password says so in password words; the right one unlocks.
        type_text(&mut gate, &mut fake, "abcd", 3);
        assert_eq!(enter(&mut gate, &mut fake, 3), Action::Redraw);
        assert_eq!(gate.view(3).notice, Notice::Incorrect);
        type_text(&mut gate, &mut fake, "door", 4);
        assert_eq!(enter(&mut gate, &mut fake, 4), Action::Granted);
        assert_eq!(fake.kinds, [Kind::Pin, Kind::Password, Kind::Password]);
        // Back to the PIN tile, and the next lock starts with the PIN.
        gate.reset(Purpose::Unlock, &mut fake, 5);
        gate.tick(&mut fake, 5);
        assert_eq!(gate.view(5).kind, Kind::Pin);
        click(&mut gate, &mut fake, inside(layout.switch), 5);
        click(&mut gate, &mut fake, inside(layout.choices[1]), 5);
        click(&mut gate, &mut fake, inside(layout.switch), 5);
        assert_eq!(
            click(&mut gate, &mut fake, inside(layout.choices[0]), 5),
            Action::Redraw
        );
        assert_eq!(gate.view(5).kind, Kind::Pin);
        // Clicking "Sign-in options" again closes the tiles.
        click(&mut gate, &mut fake, inside(layout.switch), 5);
        assert_eq!(
            click(&mut gate, &mut fake, inside(layout.switch), 5),
            Action::Redraw
        );
        assert!(!gate.view(5).options);
    }

    #[test]
    fn two_kinds_keyboard_reaches_sign_in_options_and_the_throttle_is_shared() {
        let mut fake = Fake::both(b"door", b"2468");
        let mut gate = Gate::new(Purpose::Unlock);
        gate.start(&mut fake, 0);
        gate.tick(&mut fake, 0);
        // Tab: box -> "Sign-in options"; Enter opens the tiles.
        assert_eq!(press(&mut gate, &mut fake, TAB, 0, 0), Action::Redraw);
        assert_eq!(gate.view(0).focus, Focus::Options);
        // Typing and Backspace do nothing while a link has focus.
        type_text(&mut gate, &mut fake, "12", 0);
        assert_eq!(press(&mut gate, &mut fake, 0x0e, 0, 0), Action::None);
        assert!(gate.view(0).entry.is_empty());
        assert_eq!(press(&mut gate, &mut fake, 0x1c, 0, 0), Action::Redraw);
        assert!(gate.view(0).options);
        assert!(fake.sent.is_empty(), "Enter on a link submits nothing");
        // Tab reaches the tiles; the arrows move between them.
        press(&mut gate, &mut fake, TAB, 0, 0);
        assert_eq!(gate.view(0).focus, Focus::Choice(Kind::Pin));
        press(&mut gate, &mut fake, RIGHT, 0, 0);
        assert_eq!(gate.view(0).focus, Focus::Choice(Kind::Password));
        press(&mut gate, &mut fake, LEFT, 0, 0);
        press(&mut gate, &mut fake, RIGHT, 0, 0);
        // Space chooses the password and returns to the box.
        assert_eq!(press(&mut gate, &mut fake, SPACE, 0, 0), Action::Redraw);
        let view = gate.view(0);
        assert_eq!(
            (view.kind, view.focus, view.options),
            (Kind::Password, Focus::Entry, false)
        );
        // Shift+Tab walks backwards: box -> "Sign-in options" (tiles closed).
        press(&mut gate, &mut fake, TAB, SHIFT, 0);
        assert_eq!(gate.view(0).focus, Focus::Options);
        press(&mut gate, &mut fake, TAB, 0, 0);
        assert_eq!(gate.view(0).focus, Focus::Entry);
        // Escape closes the tiles and returns to the box.
        press(&mut gate, &mut fake, TAB, 0, 0);
        press(&mut gate, &mut fake, SPACE, 0, 0);
        press(&mut gate, &mut fake, TAB, 0, 0);
        assert_eq!(gate.view(0).focus, Focus::Choice(Kind::Pin));
        press(&mut gate, &mut fake, ESC, 0, 0);
        let view = gate.view(0);
        assert_eq!(
            (view.focus, view.options, view.kind),
            (Focus::Entry, false, Kind::Password)
        );
        // Five wrong passwords start the throttle the service shares.
        for _ in 0..credential::FREE_ATTEMPTS {
            type_text(&mut gate, &mut fake, "abcd", 10);
            enter(&mut gate, &mut fake, 10);
        }
        assert_eq!(gate.view(10).notice, Notice::Throttled);
        // Switching to the PIN keeps the countdown; the right PIN is refused
        // locally until it ends.
        press(&mut gate, &mut fake, TAB, 0, 10);
        press(&mut gate, &mut fake, 0x1c, 0, 10);
        press(&mut gate, &mut fake, TAB, 0, 10);
        press(&mut gate, &mut fake, 0x1c, 0, 10);
        let view = gate.view(10);
        assert_eq!((view.kind, view.notice), (Kind::Pin, Notice::Throttled));
        let sent = fake.sent.len();
        type_text(&mut gate, &mut fake, "2468", 20);
        assert_eq!(enter(&mut gate, &mut fake, 20), Action::Redraw);
        assert_eq!(fake.sent.len(), sent);
        let later = 10 + credential::FIRST_DELAY_TICKS;
        gate.tick(&mut fake, later);
        type_text(&mut gate, &mut fake, "2468", later);
        assert_eq!(enter(&mut gate, &mut fake, later), Action::Granted);
    }

    #[test]
    fn sign_in_options_paint_and_hit_inside_the_smallest_output() {
        let l = GateLayout::new(640, 480, Page::SignIn);
        assert!(l.notice_top + 20 <= l.switch.y());
        assert!(l.switch.bottom() < l.choices[0].y());
        assert!(l.choices[0].right() < l.choices[1].x());
        assert!(l.choices[1].bottom() < 480);
        assert_eq!(l.choices[0].x() + l.choices[1].right(), 640);
        assert_eq!(
            l.option_hit(false, l.switch.x() + 1, l.switch.y() + 1),
            Some(Hit::Options)
        );
        assert_eq!(
            l.option_hit(false, l.choices[0].x() + 1, l.choices[0].y() + 1),
            None
        );
        for (rect, kind) in l.choices.into_iter().zip(CHOICES) {
            assert_eq!(
                l.option_hit(true, rect.x() + 1, rect.y() + 1),
                Some(Hit::Choice(kind))
            );
        }
        for factor in 1..=3 {
            let scale = door_graphics_core::Scale::from_factor(factor).unwrap();
            let (w, h) = (640 * factor, 480 * factor);
            let fonts = FontAtlas::windows_reference_scaled(factor).unwrap();
            let paint = |options: bool, focus: Focus| {
                let mut pixels = alloc::vec![90u8; (w * h * 4) as usize];
                let mut canvas = Canvas::new(&mut pixels, w, h, w as usize * 4).unwrap();
                canvas.set_scale(scale).unwrap();
                let view = View {
                    purpose: Purpose::Unlock,
                    page: Page::SignIn,
                    kind: Kind::Pin,
                    entry: Secret::EMPTY,
                    confirm: Secret::EMPTY,
                    confirm_focus: false,
                    reveal: false,
                    notice: Notice::None,
                    retry_seconds: 0,
                    busy: false,
                    both: true,
                    options,
                    focus,
                    users: None,
                    selected: 1001,
                };
                draw_gate(&mut canvas, &fonts, &view);
                pixels
            };
            let closed = paint(false, Focus::Entry);
            let open = paint(true, Focus::Entry);
            let focused = paint(true, Focus::Choice(Kind::Password));
            let ring = paint(false, Focus::Options);
            // Opening adds the tiles, focus adds a ring: all differ.
            assert_ne!(closed, open);
            assert_ne!(open, focused);
            assert_ne!(closed, ring);
            // Nothing is painted below the tiles.
            let tiles = crate::taskbar::physical(l.choices[1], scale);
            let below = (tiles.bottom() + 4 * factor as i32) as usize;
            let row = w as usize * 4;
            assert!(
                open[below * row..]
                    .iter()
                    .zip(&closed[below * row..])
                    .all(|(a, b)| a == b)
            );
        }
    }
}
