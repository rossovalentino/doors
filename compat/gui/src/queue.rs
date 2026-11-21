//! Per-thread message queues: posted and input messages, cross-thread sent
//! messages with replies, abandonment and timeouts, the quit flag, timers,
//! WM_PAINT synthesis, InSendMessage accounting and the hung-application
//! rule. The model is single-threaded data; user32 keeps it behind its
//! process lock, wakes waiting threads, and never holds the lock while a
//! window procedure runs.
//!
//! Retrieval order (Get/PeekMessage): posted messages, the quit message,
//! input messages, WM_PAINT, then WM_TIMER. Incoming sent messages are
//! dispatched by user32 before any retrieval and while a sender waits.
use crate::keyboard::KeyState;
use crate::msg::wm;
use crate::{Error, Point, Result};
use alloc::collections::VecDeque;
use alloc::vec::Vec;

/// GetQueueStatus / MsgWaitForMultipleObjects kinds.
pub mod qs {
    pub const KEY: u32 = 0x0001;
    pub const MOUSEMOVE: u32 = 0x0002;
    pub const MOUSEBUTTON: u32 = 0x0004;
    pub const POSTMESSAGE: u32 = 0x0008;
    pub const TIMER: u32 = 0x0010;
    pub const PAINT: u32 = 0x0020;
    pub const SENDMESSAGE: u32 = 0x0040;
    pub const HOTKEY: u32 = 0x0080;
    pub const ALLPOSTMESSAGE: u32 = 0x0100;
}
/// The QS_* kind of a queued input message.
fn input_kind(message: u32) -> u32 {
    match message {
        wm::KEYFIRST..=wm::KEYLAST => qs::KEY,
        wm::MOUSEMOVE | wm::NCMOUSEMOVE => qs::MOUSEMOVE,
        wm::MOUSEFIRST..=wm::MOUSELAST | 0x00a1..=0x00ad => qs::MOUSEBUTTON,
        _ => qs::POSTMESSAGE | qs::ALLPOSTMESSAGE,
    }
}
static NO_KEYS: KeyState = KeyState::new();

pub const MAX_THREADS: usize = 64;
pub const MAX_POSTED: usize = 10_000;
pub const MAX_SENDS: usize = 1024;
pub const MAX_TIMERS: usize = 1024;
/// A thread that has not retrieved messages for this long while not
/// waiting for input is hung (`IsHungAppWindow`).
pub const HUNG_MS: u64 = 5000;
/// First identifier issued to `SetTimer(NULL, 0, ...)` timers.
pub const THREAD_TIMER_BASE: u64 = 0x7ffe;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Message {
    pub hwnd: u64,
    pub message: u32,
    pub wparam: u64,
    pub lparam: i64,
    pub time: u32,
    pub pt: Point,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SendState {
    Queued,
    Running,
    Done(i64),
    /// The sender stopped waiting while the receiver was running it.
    Abandoned,
}

#[derive(Debug)]
struct Send {
    id: u64,
    from: u32,
    to: u32,
    message: Message,
    state: SendState,
    /// Marshalled pointer parameters (WM_GETTEXT, WM_SETTEXT, ...): the
    /// receiver reads and writes this record-owned copy, never the sender's
    /// memory, so a sender that timed out cannot leave it a dangling
    /// pointer. Freed with the record, after the receiver finished.
    buffer: Option<Vec<u8>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Timer {
    pub thread: u32,
    pub hwnd: u64,
    pub id: u64,
    pub interval: u32,
    pub due: u64,
    pub procedure: u64,
}

#[derive(Debug, Default)]
pub struct Thread {
    pub id: u32,
    posted: VecDeque<Message>,
    input: VecDeque<Message>,
    incoming: VecDeque<u64>,
    quit: Option<i32>,
    pub last_retrieval: u64,
    /// Inside GetMessage/WaitMessage/MsgWaitForMultipleObjects or a send wait.
    pub waiting: bool,
    /// Cross-thread sends being processed (innermost last).
    receiving: Vec<u64>,
    /// DLL wake handle (an event), opaque here.
    pub wake: u64,
    /// The thread has slept waiting for input (GetMessage, WaitMessage,
    /// MsgWaitForMultipleObjects): its wake mask then keeps QS_INPUT.
    input_wait: bool,
    /// Synchronous key state (`GetKeyState`): an input key message applies
    /// its transition when this thread retrieves it, so a burst of device
    /// events (Ctrl down, S down, S up, Ctrl up) still reads Ctrl down while
    /// the S key message is processed.
    pub keys: KeyState,
    /// Time and cursor position of the last retrieved message
    /// (`GetMessageTime`, `GetMessagePos`).
    pub message_time: u32,
    pub message_pos: Point,
    /// QS_* kinds added since the last GetQueueStatus or retrieval.
    added: u32,
    /// Due QS_TIMER/QS_PAINT already reported or retrieved past.
    due_seen: u32,
}

/// Which messages a retrieval accepts (hwnd and range filters).
pub struct Filter<'a> {
    pub accept: &'a dyn Fn(&Message) -> bool,
    pub min: u32,
    pub max: u32,
}
impl Filter<'_> {
    fn range(&self, message: u32) -> bool {
        (self.min == 0 && self.max == 0) || (self.min <= message && message <= self.max)
    }
    fn ok(&self, m: &Message) -> bool {
        self.range(m.message) && (self.accept)(m)
    }
}

#[derive(Default)]
pub struct Queues {
    threads: Vec<Thread>,
    sends: Vec<Send>,
    timers: Vec<Timer>,
    next_send: u64,
    next_thread_timer: u64,
}

impl Queues {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn thread(&self, id: u32) -> Option<&Thread> {
        self.threads.iter().find(|t| t.id == id)
    }
    fn get_mut(&mut self, id: u32) -> Option<&mut Thread> {
        self.threads.iter_mut().find(|t| t.id == id)
    }
    /// The queue of `id`, created on first use.
    pub fn ensure(&mut self, id: u32, now: u64) -> Result<&mut Thread> {
        if let Some(i) = self.threads.iter().position(|t| t.id == id) {
            return Ok(&mut self.threads[i]);
        }
        crate::try_push(
            &mut self.threads,
            Thread {
                id,
                last_retrieval: now,
                input_wait: false,
                ..Thread::default()
            },
            MAX_THREADS,
        )?;
        Ok(self.threads.last_mut().unwrap())
    }
    /// Remove a thread's queue at thread exit (DLL_THREAD_DETACH), freeing
    /// its slot for later threads. Pending sends to it complete with zero
    /// (the senders to wake are returned); its own queued sends are
    /// withdrawn from their receivers, and a send of it still running keeps
    /// its record (and marshalled buffer) until the receiver finishes.
    pub fn remove_thread(&mut self, id: u32) -> Vec<u32> {
        self.threads.retain(|t| t.id != id);
        self.timers.retain(|t| t.thread != id);
        let mut woken = Vec::new();
        for s in &mut self.sends {
            if s.to == id && matches!(s.state, SendState::Queued | SendState::Running) {
                s.state = SendState::Done(0);
                if s.from != id {
                    woken.push(s.from);
                }
            } else if s.from == id && s.state == SendState::Running {
                s.state = SendState::Abandoned;
            }
        }
        let withdrawn: Vec<u64> = self
            .sends
            .iter()
            .filter(|s| s.from == id && s.state != SendState::Abandoned)
            .map(|s| s.id)
            .collect();
        for t in &mut self.threads {
            t.incoming.retain(|x| !withdrawn.contains(x));
        }
        self.sends.retain(|s| !withdrawn.contains(&s.id));
        woken
    }
    /// Synchronous key state of `thread` (all keys up when it has no queue).
    pub fn key_state(&self, thread: u32) -> &KeyState {
        self.thread(thread).map_or(&NO_KEYS, |t| &t.keys)
    }
    /// A key transition that reaches no message (no focus window) still
    /// updates the thread's synchronous state at once.
    pub fn apply_key(&mut self, thread: u32, key: u8, down: bool) {
        if let Some(t) = self.get_mut(thread) {
            t.keys.apply(key, down);
        }
    }
    pub fn wake_handle(&self, id: u32) -> Option<u64> {
        self.thread(id).map(|t| t.wake)
    }
    pub fn post(&mut self, thread: u32, m: Message) -> Result<()> {
        let t = self.get_mut(thread).ok_or(Error::InvalidHandle)?;
        if t.posted.len() >= MAX_POSTED {
            return Err(Error::Quota);
        }
        t.posted.try_reserve(1).map_err(|_| Error::Quota)?;
        t.posted.push_back(m);
        t.added |= qs::POSTMESSAGE | qs::ALLPOSTMESSAGE;
        Ok(())
    }
    pub fn post_input(&mut self, thread: u32, m: Message) -> Result<()> {
        let t = self.get_mut(thread).ok_or(Error::InvalidHandle)?;
        if t.input.len() >= MAX_POSTED {
            return Err(Error::Quota);
        }
        t.input.try_reserve(1).map_err(|_| Error::Quota)?;
        t.input.push_back(m);
        t.added |= input_kind(m.message);
        Ok(())
    }
    pub fn post_quit(&mut self, thread: u32, code: i32) -> Result<()> {
        let t = self.get_mut(thread).ok_or(Error::InvalidHandle)?;
        t.quit = Some(code);
        t.added |= qs::POSTMESSAGE | qs::ALLPOSTMESSAGE;
        Ok(())
    }
    /// Drop queued work for a destroyed window.
    pub fn forget_window(&mut self, hwnd: u64) {
        for t in &mut self.threads {
            t.posted.retain(|m| m.hwnd != hwnd);
            t.input.retain(|m| m.hwnd != hwnd);
        }
        self.timers.retain(|t| t.hwnd != hwnd);
    }

    // ---- cross-thread sends ------------------------------------------------

    pub fn begin_send(&mut self, from: u32, to: u32, m: Message) -> Result<u64> {
        self.begin_send_buffered(from, to, m, None)
    }
    /// A send whose pointer parameter was marshalled into `buffer` (the
    /// caller points `m`'s parameters into it; its heap block never moves).
    pub fn begin_send_buffered(
        &mut self,
        from: u32,
        to: u32,
        m: Message,
        buffer: Option<Vec<u8>>,
    ) -> Result<u64> {
        if self.sends.len() >= MAX_SENDS {
            return Err(Error::Quota);
        }
        let t = self.get_mut(to).ok_or(Error::InvalidHandle)?;
        t.incoming.try_reserve(1).map_err(|_| Error::Quota)?;
        self.sends.try_reserve(1).map_err(|_| Error::Quota)?;
        self.next_send += 1;
        let id = self.next_send;
        let t = self.get_mut(to).unwrap();
        t.incoming.push_back(id);
        t.added |= qs::SENDMESSAGE;
        self.sends.push(Send {
            id,
            from,
            to,
            message: m,
            state: SendState::Queued,
            buffer,
        });
        Ok(id)
    }
    pub fn has_incoming(&self, thread: u32) -> bool {
        self.thread(thread).is_some_and(|t| !t.incoming.is_empty())
    }
    /// Take the next sent message for `thread` (it becomes Running).
    pub fn take_incoming(&mut self, thread: u32) -> Option<(u64, Message)> {
        let t = self.threads.iter_mut().find(|t| t.id == thread)?;
        let id = t.incoming.pop_front()?;
        t.receiving.push(id);
        let s = self.sends.iter_mut().find(|s| s.id == id)?;
        s.state = SendState::Running;
        Some((id, s.message))
    }
    /// Complete a send; returns the sender to wake (None when abandoned or
    /// already replied through ReplyMessage).
    pub fn reply(&mut self, id: u64, result: i64) -> Option<u32> {
        let i = self.sends.iter().position(|s| s.id == id)?;
        match self.sends[i].state {
            SendState::Running => {
                self.sends[i].state = SendState::Done(result);
                Some(self.sends[i].from)
            }
            SendState::Abandoned => {
                self.sends.remove(i);
                None
            }
            _ => None,
        }
    }
    /// The receiver finished the procedure for `id`.
    pub fn finish(&mut self, thread: u32, id: u64, result: i64) -> Option<u32> {
        if let Some(t) = self.get_mut(thread)
            && let Some(p) = t.receiving.iter().rposition(|&x| x == id)
        {
            t.receiving.remove(p);
        }
        self.reply(id, result)
    }
    /// ReplyMessage: reply to the innermost send early.
    pub fn reply_current(&mut self, thread: u32, result: i64) -> Option<u32> {
        let id = *self.thread(thread)?.receiving.last()?;
        self.reply(id, result)
    }
    /// The sender's view: the result once done (and the record released).
    pub fn take_result(&mut self, id: u64) -> Option<i64> {
        self.take_reply(id).map(|(r, _)| r)
    }
    /// The result and the marshalled buffer, once done.
    pub fn take_reply(&mut self, id: u64) -> Option<(i64, Option<Vec<u8>>)> {
        let i = self.sends.iter().position(|s| s.id == id)?;
        if let SendState::Done(r) = self.sends[i].state {
            return Some((r, self.sends.remove(i).buffer));
        }
        None
    }
    /// The sender gives up (timeout): a queued send is withdrawn and is
    /// never delivered; a running one is marked abandoned. Returns true when
    /// the message was withdrawn before delivery.
    pub fn abandon(&mut self, id: u64) -> bool {
        let Some(i) = self.sends.iter().position(|s| s.id == id) else {
            return false;
        };
        match self.sends[i].state {
            SendState::Queued => {
                let to = self.sends[i].to;
                if let Some(t) = self.get_mut(to) {
                    t.incoming.retain(|&x| x != id);
                }
                self.sends.remove(i);
                true
            }
            SendState::Running => {
                self.sends[i].state = SendState::Abandoned;
                false
            }
            SendState::Done(_) | SendState::Abandoned => {
                self.sends.remove(i);
                false
            }
        }
    }
    pub fn send_state(&self, id: u64) -> Option<SendState> {
        self.sends.iter().find(|s| s.id == id).map(|s| s.state)
    }
    /// InSendMessage: processing a message sent by another thread.
    pub fn in_send(&self, thread: u32) -> bool {
        self.thread(thread).is_some_and(|t| !t.receiving.is_empty())
    }
    /// The sending thread of the innermost send (InSendMessageEx details).
    pub fn current_sender(&self, thread: u32) -> Option<u32> {
        let id = *self.thread(thread)?.receiving.last()?;
        self.sends.iter().find(|s| s.id == id).map(|s| s.from)
    }
    /// ReplyMessage already answered the innermost send (ISMEX_REPLIED).
    pub fn current_replied(&self, thread: u32) -> bool {
        let Some(&id) = self.thread(thread).and_then(|t| t.receiving.last()) else {
            return false;
        };
        match self.sends.iter().find(|s| s.id == id) {
            // The sender already took the early reply.
            None => true,
            Some(s) => matches!(s.state, SendState::Done(_)),
        }
    }

    // ---- timers -------------------------------------------------------------

    /// SetTimer. `id` 0 with no window allocates a fresh identifier.
    pub fn set_timer(
        &mut self,
        thread: u32,
        hwnd: u64,
        id: u64,
        interval: u32,
        procedure: u64,
        now: u64,
    ) -> Result<u64> {
        let interval = interval.clamp(10, 0x7fff_ffff);
        let id = if hwnd == 0
            && (id == 0
                || !self
                    .timers
                    .iter()
                    .any(|t| t.hwnd == 0 && t.id == id && t.thread == thread))
        {
            loop {
                self.next_thread_timer += 1;
                let candidate = THREAD_TIMER_BASE + self.next_thread_timer;
                if !self.timers.iter().any(|t| t.hwnd == 0 && t.id == candidate) {
                    break candidate;
                }
            }
        } else {
            id
        };
        if let Some(t) = self
            .timers
            .iter_mut()
            .find(|t| t.hwnd == hwnd && t.id == id)
        {
            t.interval = interval;
            t.due = now + u64::from(interval);
            t.procedure = procedure;
            return Ok(id);
        }
        crate::try_push(
            &mut self.timers,
            Timer {
                thread,
                hwnd,
                id,
                interval,
                due: now + u64::from(interval),
                procedure,
            },
            MAX_TIMERS,
        )?;
        Ok(id)
    }
    /// KillTimer; thread timers belong to their creating thread.
    pub fn kill_timer(&mut self, thread: u32, hwnd: u64, id: u64) -> bool {
        let before = self.timers.len();
        self.timers
            .retain(|t| !(t.hwnd == hwnd && t.id == id && (hwnd != 0 || t.thread == thread)));
        before != self.timers.len()
    }
    pub fn next_due(&self, thread: u32) -> Option<u64> {
        self.timers
            .iter()
            .filter(|t| t.thread == thread)
            .map(|t| t.due)
            .min()
    }
    pub fn timers(&self) -> &[Timer] {
        &self.timers
    }

    // ---- retrieval ------------------------------------------------------------

    /// Get/PeekMessage retrieval. `paint` names a window of this thread that
    /// needs WM_PAINT and passes the filter. Records the retrieval time, the
    /// removed message's time and position (`GetMessageTime`) and applies a
    /// removed input key message to the synchronous key state.
    pub fn next(
        &mut self,
        thread: u32,
        filter: &Filter,
        remove: bool,
        now: u64,
        paint: &mut dyn FnMut(&Filter) -> Option<u64>,
    ) -> Option<Message> {
        let m = self.next_inner(thread, filter, remove, now, paint);
        if let Some(t) = self.get_mut(thread) {
            t.added = 0;
            t.due_seen = qs::TIMER | qs::PAINT;
            if let Some(m) = m.filter(|_| remove) {
                t.message_time = m.time;
                // Synthesized messages carry no cursor position of their own.
                if !matches!(m.message, wm::PAINT | wm::TIMER | wm::QUIT) {
                    t.message_pos = m.pt;
                }
            }
        }
        m
    }
    fn next_inner(
        &mut self,
        thread: u32,
        filter: &Filter,
        remove: bool,
        now: u64,
        paint: &mut dyn FnMut(&Filter) -> Option<u64>,
    ) -> Option<Message> {
        let i = self.threads.iter().position(|t| t.id == thread)?;
        self.threads[i].last_retrieval = now;
        let time = now as u32;
        let t = &mut self.threads[i];
        if let Some(p) = t.posted.iter().position(|m| filter.ok(m)) {
            return Some(if remove {
                t.posted.remove(p).unwrap()
            } else {
                t.posted[p]
            });
        }
        if let Some(code) = t.quit
            && filter.range(wm::QUIT)
        {
            if remove {
                t.quit = None;
            }
            return Some(Message {
                message: wm::QUIT,
                wparam: code as u64,
                time,
                ..Message::default()
            });
        }
        if let Some(p) = t.input.iter().position(|m| filter.ok(m)) {
            return Some(if remove {
                let m = t.input.remove(p).unwrap();
                if let Some((key, down)) =
                    crate::keyboard::transition(m.message, m.wparam, m.lparam)
                {
                    t.keys.apply(key, down);
                }
                m
            } else {
                t.input[p]
            });
        }
        if filter.range(wm::PAINT)
            && let Some(hwnd) = paint(filter)
        {
            return Some(Message {
                hwnd,
                message: wm::PAINT,
                time,
                ..Message::default()
            });
        }
        if filter.range(wm::TIMER) {
            let due = self
                .timers
                .iter_mut()
                .filter(|x| x.thread == thread && x.due <= now)
                .find(|x| {
                    (filter.accept)(&Message {
                        hwnd: x.hwnd,
                        message: wm::TIMER,
                        ..Message::default()
                    })
                });
            if let Some(x) = due {
                if remove {
                    x.due = now + u64::from(x.interval);
                }
                return Some(Message {
                    hwnd: x.hwnd,
                    message: wm::TIMER,
                    wparam: x.id,
                    lparam: x.procedure as i64,
                    time,
                    ..Message::default()
                });
            }
        }
        None
    }
    /// GetQueueStatus-style summary: (posted, input, sent, quit).
    pub fn pending(&self, thread: u32) -> (bool, bool, bool, bool) {
        self.thread(thread)
            .map_or((false, false, false, false), |t| {
                (
                    !t.posted.is_empty(),
                    !t.input.is_empty(),
                    !t.incoming.is_empty(),
                    t.quit.is_some(),
                )
            })
    }
    /// GetQueueStatus: high word the QS_* kinds now in the queue, low word
    /// those added since the last GetQueueStatus, GetMessage or PeekMessage,
    /// both masked by `flags`; the queried kinds stop counting as new.
    /// `paint`: a window of the thread needs WM_PAINT.
    pub fn queue_status(&mut self, thread: u32, flags: u32, now: u64, paint: bool) -> u32 {
        let timer = self
            .timers
            .iter()
            .any(|x| x.thread == thread && x.due <= now);
        let Some(t) = self.get_mut(thread) else {
            return 0;
        };
        let mut current = t.input.iter().fold(0, |k, m| k | input_kind(m.message));
        if !t.posted.is_empty() || t.quit.is_some() {
            current |= qs::POSTMESSAGE | qs::ALLPOSTMESSAGE;
        }
        if !t.incoming.is_empty() {
            current |= qs::SENDMESSAGE;
        }
        if timer {
            current |= qs::TIMER;
        }
        if paint {
            current |= qs::PAINT;
        }
        let flags = flags & 0x05ff;
        // Timers and paint become due rather than being added: new until
        // reported (or retrieved past) once.
        let due = current & (qs::TIMER | qs::PAINT);
        t.due_seen &= due;
        let new = (t.added | (due & !t.due_seen)) & current;
        t.due_seen |= due & flags;
        t.added &= !flags;
        ((current & flags) << 16) | (new & flags)
    }
    pub fn set_waiting(&mut self, thread: u32, waiting: bool, now: u64) {
        if let Some(t) = self.get_mut(thread) {
            t.waiting = waiting;
            // Entering or leaving a wait for input both count as responsive.
            t.last_retrieval = now;
        }
    }
    /// A wait for input (not a send wait): Windows leaves QS_INPUT in the
    /// thread's wake mask afterwards.
    pub fn note_input_wait(&mut self, thread: u32) {
        if let Some(t) = self.get_mut(thread) {
            t.input_wait = true;
        }
    }
    /// SMTO_ABORTIFHUNG gives up at once only on a hung receiver whose wake
    /// mask holds no input: one that never slept waiting for input. A thread
    /// stuck inside a message it retrieved is waited for until the timeout.
    /// (Windows reference: wingui-selftest's never-pumping hung thread
    /// aborts fast; wingui-hang's target stuck in WM_STOP times out.)
    pub fn abandons_send(&self, thread: u32, now: u64) -> bool {
        self.thread(thread).is_some_and(|t| !t.input_wait) && self.is_hung(thread, now)
    }
    /// The five-second hung rule.
    pub fn is_hung(&self, thread: u32, now: u64) -> bool {
        self.thread(thread)
            .is_some_and(|t| !t.waiting && now.saturating_sub(t.last_retrieval) >= HUNG_MS)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn any() -> Filter<'static> {
        Filter {
            accept: &|_| true,
            min: 0,
            max: 0,
        }
    }
    fn msg(hwnd: u64, message: u32, wparam: u64) -> Message {
        Message {
            hwnd,
            message,
            wparam,
            ..Message::default()
        }
    }
    #[test]
    fn retrieval_order_posted_quit_input_paint_timer() {
        let mut q = Queues::new();
        q.ensure(1, 0).unwrap();
        q.set_timer(1, 5, 7, 10, 0, 0).unwrap();
        q.post_input(1, msg(5, wm::KEYDOWN, 1)).unwrap();
        q.post(1, msg(5, wm::APP + 10, 1)).unwrap();
        q.post(1, msg(5, wm::APP + 10, 2)).unwrap();
        q.post_quit(1, 4).unwrap();
        let mut dirty = true;
        let mut paint = |_: &Filter| dirty.then_some(5u64);
        let mut seen = Vec::new();
        while let Some(m) = q.next(1, &any(), true, 20, &mut paint) {
            seen.push((m.message, m.wparam));
            if m.message == wm::PAINT {
                break;
            }
        }
        assert_eq!(
            seen,
            [
                (wm::APP + 10, 1),
                (wm::APP + 10, 2),
                (wm::QUIT, 4),
                (wm::KEYDOWN, 1),
                (wm::PAINT, 0)
            ]
        );
        dirty = false;
        let mut paint = |_: &Filter| dirty.then_some(5u64);
        let t = q.next(1, &any(), true, 20, &mut paint).unwrap();
        assert_eq!((t.message, t.wparam), (wm::TIMER, 7));
        assert!(
            q.next(1, &any(), true, 21, &mut paint).is_none(),
            "timer rearmed"
        );
        assert!(q.next(1, &any(), true, 30, &mut paint).is_some());
    }
    #[test]
    fn filters_and_noremove() {
        let mut q = Queues::new();
        q.ensure(1, 0).unwrap();
        q.post(1, msg(5, 0x400, 0)).unwrap();
        q.post(1, msg(6, 0x401, 0)).unwrap();
        let only6 = Filter {
            accept: &|m| m.hwnd == 6,
            min: 0,
            max: 0,
        };
        let mut none = |_: &Filter| None;
        assert_eq!(q.next(1, &only6, false, 1, &mut none).unwrap().hwnd, 6);
        assert_eq!(q.next(1, &only6, true, 1, &mut none).unwrap().hwnd, 6);
        assert!(q.next(1, &only6, true, 1, &mut none).is_none());
        let range = Filter {
            accept: &|_| true,
            min: 0x401,
            max: 0x500,
        };
        assert!(q.next(1, &range, true, 1, &mut none).is_none());
        q.forget_window(5);
        assert!(q.next(1, &any(), true, 1, &mut none).is_none());
        assert_eq!(q.post(9, msg(0, 1, 0)), Err(Error::InvalidHandle));
    }
    #[test]
    fn cross_thread_send_reply_nesting_and_insend() {
        let mut q = Queues::new();
        q.ensure(1, 0).unwrap();
        q.ensure(2, 0).unwrap();
        let outer = q.begin_send(1, 2, msg(20, 0x404, 7)).unwrap();
        assert!(q.has_incoming(2));
        let (id, m) = q.take_incoming(2).unwrap();
        assert_eq!((id, m.wparam), (outer, 7));
        assert!(q.in_send(2) && !q.in_send(1));
        assert_eq!(q.current_sender(2), Some(1));
        // The receiver calls back into the waiting sender.
        let inner = q.begin_send(2, 1, msg(10, 0x405, 7)).unwrap();
        let (iid, _) = q.take_incoming(1).unwrap();
        assert!(q.in_send(1));
        assert_eq!(q.finish(1, iid, 107), Some(2));
        assert!(!q.in_send(1));
        assert_eq!(q.take_result(inner), Some(107));
        assert_eq!(q.take_result(outer), None, "outer still running");
        assert_eq!(q.finish(2, outer, 1107), Some(1));
        assert_eq!(q.take_result(outer), Some(1107));
        assert_eq!(q.send_state(outer), None, "released");
    }
    #[test]
    fn timeouts_withdraw_queued_sends_and_abandon_running_ones() {
        let mut q = Queues::new();
        q.ensure(1, 0).unwrap();
        q.ensure(2, 0).unwrap();
        let a = q.begin_send(1, 2, msg(20, 0x406, 0)).unwrap();
        assert!(q.abandon(a), "withdrawn before delivery");
        assert!(q.take_incoming(2).is_none(), "never delivered late");
        let b = q.begin_send(1, 2, msg(20, 0x406, 1)).unwrap();
        q.take_incoming(2).unwrap();
        assert!(!q.abandon(b));
        assert_eq!(q.finish(2, b, 5), None, "nobody waits");
        assert_eq!(q.send_state(b), None);
        let c = q.begin_send(1, 2, msg(20, 0x406, 2)).unwrap();
        q.take_incoming(2).unwrap();
        assert_eq!(q.reply_current(2, 9), Some(1), "ReplyMessage");
        assert_eq!(q.finish(2, c, 10), None, "already replied");
        assert_eq!(q.take_result(c), Some(9));
        // A thread exit completes sends addressed to it.
        let d = q.begin_send(1, 2, msg(20, 0x406, 3)).unwrap();
        assert_eq!(q.remove_thread(2), [1]);
        assert_eq!(q.take_result(d), Some(0));
    }
    #[test]
    fn abort_if_hung_skips_only_receivers_that_never_waited_for_input() {
        let mut q = Queues::new();
        q.ensure(3, 1000).unwrap();
        q.ensure(4, 1000).unwrap();
        // Never pumped and silent for five seconds: abandoned at once.
        assert!(!q.abandons_send(3, 5999));
        assert!(q.abandons_send(3, 6000));
        // Stuck inside a retrieved message after waiting for input: hung for
        // IsHungAppWindow, but a send still waits until its timeout.
        q.set_waiting(4, true, 1500);
        q.note_input_wait(4);
        q.set_waiting(4, false, 2000);
        assert!(q.is_hung(4, 7000));
        assert!(!q.abandons_send(4, 7000));
    }
    #[test]
    fn hung_rule_and_timer_ids() {
        let mut q = Queues::new();
        q.ensure(3, 1000).unwrap();
        assert!(!q.is_hung(3, 5999));
        assert!(q.is_hung(3, 6000));
        q.set_waiting(3, true, 7000);
        assert!(!q.is_hung(3, 20_000), "waiting for input is not hung");
        q.set_waiting(3, false, 20_000);
        assert!(!q.is_hung(3, 20_100));
        let mut none = |_: &Filter| None;
        q.next(3, &any(), false, 30_000, &mut none);
        assert!(!q.is_hung(3, 34_999));
        let a = q.set_timer(3, 0, 0, 20, 0x1234, 0).unwrap();
        let b = q.set_timer(3, 0, 0, 20, 0x1234, 0).unwrap();
        assert!(a != 0 && b != 0 && a != b);
        assert_eq!(q.set_timer(3, 9, 7, 5, 0, 0), Ok(7));
        assert_eq!(
            q.timers().iter().find(|t| t.id == 7).unwrap().interval,
            10,
            "clamped"
        );
        assert!(q.kill_timer(3, 9, 7));
        assert!(!q.kill_timer(3, 9, 7));
        assert!(!q.kill_timer(4, 0, a), "foreign thread timer");
        assert!(q.kill_timer(3, 0, a));
        assert_eq!(q.next_due(3), Some(20));
        for _ in 0..MAX_THREADS - 1 {
            let n = q.threads.len() as u32 + 100;
            q.ensure(n, 0).unwrap();
        }
        assert_eq!(q.ensure(9999, 0).err(), Some(Error::Quota));
    }
    #[test]
    fn key_state_follows_retrieval_not_arrival() {
        use crate::keyboard::{key_lparam, vk};
        let mut q = Queues::new();
        q.ensure(1, 0).unwrap();
        // A QMP burst: Ctrl down, S down, S up, Ctrl up, all queued before
        // the application retrieves the first message (Ctrl+S).
        for (key, down) in [
            (vk::CONTROL, true),
            (b'S', true),
            (b'S', false),
            (vk::CONTROL, false),
        ] {
            let m = if down { wm::KEYDOWN } else { wm::KEYUP };
            let mut message = msg(5, m, u64::from(key));
            message.lparam = key_lparam(key, down, false, false);
            q.post_input(1, message).unwrap();
        }
        let mut none = |_: &Filter| None;
        let ctrl_at_each: Vec<(u64, bool)> = (0..4)
            .map(|_| {
                let m = q.next(1, &any(), true, 1, &mut none).unwrap();
                (m.wparam, q.key_state(1).down(vk::CONTROL))
            })
            .collect();
        assert_eq!(
            ctrl_at_each,
            [
                (u64::from(vk::CONTROL), true),
                (u64::from(b'S'), true),
                (u64::from(b'S'), true),
                (u64::from(vk::CONTROL), false),
            ]
        );
        assert!(q.key_state(1).down(vk::LCONTROL) || !q.key_state(1).down(vk::CONTROL));
        // Posted (not input) key messages never touch the key state.
        let mut posted = msg(5, wm::KEYDOWN, u64::from(vk::SHIFT));
        posted.lparam = key_lparam(vk::SHIFT, true, false, false);
        q.post(1, posted).unwrap();
        q.next(1, &any(), true, 2, &mut none).unwrap();
        assert!(!q.key_state(1).down(vk::SHIFT));
        assert!(!q.key_state(77).down(vk::SHIFT), "no queue: all up");
    }
    #[test]
    fn thread_exit_frees_its_slot_and_withdraws_its_sends() {
        let mut q = Queues::new();
        for id in 1..=MAX_THREADS as u32 {
            q.ensure(id, 0).unwrap();
        }
        assert_eq!(q.ensure(500, 0).err(), Some(Error::Quota));
        // Thread 2 queued a send to 3 and is running one on 4 when it exits.
        let queued = q.begin_send(2, 3, msg(9, wm::APP, 1)).unwrap();
        let running = q
            .begin_send_buffered(2, 4, msg(9, wm::GETTEXT, 8), Some(alloc::vec![0; 16]))
            .unwrap();
        assert_eq!(q.take_incoming(4).map(|(id, _)| id), Some(running));
        // Thread 1 waits on a send to 2.
        let to_dying = q.begin_send(1, 2, msg(9, wm::APP, 2)).unwrap();
        assert_eq!(q.remove_thread(2), [1]);
        assert_eq!(q.take_result(to_dying), Some(0));
        assert!(
            !q.has_incoming(3),
            "a dead sender's queued send is withdrawn"
        );
        assert_eq!(q.send_state(queued), None);
        assert_eq!(q.send_state(running), Some(SendState::Abandoned));
        // The receiver finishes into the record-owned buffer, which goes
        // with the record.
        assert_eq!(q.finish(4, running, 3), None);
        assert_eq!(q.send_state(running), None);
        assert!(!q.in_send(4));
        // The slot is reusable by a later thread lifetime.
        q.ensure(500, 0).unwrap();
    }
    #[test]
    fn marshalled_send_buffer_returns_to_the_sender_and_reply_is_reported() {
        let mut q = Queues::new();
        q.ensure(1, 0).unwrap();
        q.ensure(2, 0).unwrap();
        let id = q
            .begin_send_buffered(1, 2, msg(9, wm::GETTEXT, 4), Some(alloc::vec![0; 8]))
            .unwrap();
        assert!(q.take_reply(id).is_none());
        q.take_incoming(2).unwrap();
        assert!(q.in_send(2) && !q.current_replied(2));
        assert_eq!(q.reply_current(2, 3), Some(1));
        assert!(q.current_replied(2), "ISMEX_REPLIED after ReplyMessage");
        let (result, buffer) = q.take_reply(id).unwrap();
        assert_eq!((result, buffer.map(|b| b.len())), (3, Some(8)));
        assert!(q.current_replied(2));
        q.finish(2, id, 3);
        assert!(!q.in_send(2) && !q.current_replied(2));
    }
    #[test]
    fn queue_status_reports_current_and_new_kinds() {
        let mut q = Queues::new();
        q.ensure(1, 0).unwrap();
        let all = 0x05ff;
        assert_eq!(q.queue_status(1, all, 0, false), 0);
        q.post_input(1, msg(5, wm::KEYDOWN, 0x41)).unwrap();
        q.post_input(1, msg(5, wm::MOUSEMOVE, 0)).unwrap();
        q.post(1, msg(5, wm::APP, 0)).unwrap();
        let kinds = qs::KEY | qs::MOUSEMOVE | qs::POSTMESSAGE | qs::ALLPOSTMESSAGE;
        assert_eq!(q.queue_status(1, all, 0, false), kinds << 16 | kinds);
        assert_eq!(
            q.queue_status(1, all, 0, false),
            kinds << 16,
            "no longer new"
        );
        assert_eq!(
            q.queue_status(1, qs::KEY, 0, true),
            qs::KEY << 16,
            "masked by flags"
        );
        assert_eq!(
            q.queue_status(1, qs::PAINT, 0, true),
            qs::PAINT << 16 | qs::PAINT
        );
        assert_eq!(q.queue_status(1, qs::PAINT, 0, true), qs::PAINT << 16);
        let mut none = |_: &Filter| None;
        let m = q.next(1, &any(), true, 77, &mut none).unwrap();
        assert_eq!(q.thread(1).unwrap().message_time, m.time);
        assert_eq!(q.queue_status(1, qs::KEY, 0, false), qs::KEY << 16);
    }
}
