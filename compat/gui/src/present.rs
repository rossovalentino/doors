//! Presentation of a top-level window's back buffer to the Door window
//! service. Every window (children included) paints into its top-level's
//! private back buffer; changed rows are copied into the shared section and
//! committed. The service snapshots the committed rows and releases the
//! buffer, so a successful commit consumes the attachment: each later commit
//! must attach the buffer to the current configure epoch again first.
//! Committing without that attachment is refused, which used to leave every
//! paint after a window's first frame (typed EDIT text, status text) in the
//! back buffer only.

/// A failed service call.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fail {
    /// Busy: for an attach, the buffer is still attached (an earlier commit
    /// did not consume it); for a commit, try again later.
    WouldBlock,
    /// Refused or unavailable.
    Other,
}

/// The two window-service calls a commit needs, bound to one window,
/// buffer and configure epoch.
pub trait Service {
    fn attach(&mut self) -> Result<(), Fail>;
    /// Commit rows `top..bottom` (`Some`, a band) or the whole buffer.
    fn commit(&mut self, band: Option<(u32, u32)>) -> Result<(), Fail>;
}

/// Commit state of one top-level window's buffer.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Presenter {
    /// The current configure epoch was acknowledged: attaching is allowed.
    pub acknowledged: bool,
    /// Attached to the epoch and not yet consumed by a commit.
    pub attached: bool,
    /// A commit of this buffer succeeded; later commits may be bands.
    pub committed: bool,
    /// The last commit would have blocked: present everything again.
    pub retry: bool,
}

impl Presenter {
    /// A configure (new epoch, maybe a new buffer) arrived: nothing is
    /// attached or acknowledged and the next commit is complete.
    pub fn configured(&mut self) {
        *self = Self::default();
    }
    /// The rows to copy into the section before `commit` (`changed` rows,
    /// or all of them when no band may be used), or None when nothing can be
    /// presented now.
    pub fn rows(&self, changed: Option<(u32, u32)>, height: u32) -> Option<(u32, u32)> {
        if !self.acknowledged {
            return None;
        }
        let (top, bottom) = match changed {
            Some((t, b)) if self.committed && !self.retry => (t.min(height), b.min(height)),
            _ => (0, height),
        };
        (top < bottom).then_some((top, bottom))
    }
    /// Attach (unless an unconsumed attachment remains), then commit `rows`
    /// from `rows()` as a band when `band` allows it, else completely.
    pub fn commit(
        &mut self,
        rows: (u32, u32),
        changed: bool,
        band: bool,
        service: &mut dyn Service,
    ) -> Result<(), Fail> {
        let full = !changed || !self.committed || self.retry || !band;
        self.retry = false;
        if !self.attached {
            match service.attach() {
                Ok(()) | Err(Fail::WouldBlock) => self.attached = true,
                Err(e) => return Err(e),
            }
        }
        let result = service.commit(if full { None } else { Some(rows) });
        match result {
            Ok(()) => {
                self.committed = true;
                self.attached = false;
            }
            Err(Fail::WouldBlock) => self.retry = true,
            Err(Fail::Other) => self.attached = false,
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::edit::Edit;
    use crate::gdi::{External, Gdi, LogFont, TextBackend, TextMetrics};
    use crate::handles::Handle;
    use crate::msg::{es, stock, ws};
    use crate::raster::{self, Canvas};
    use crate::region::Region;
    use crate::window::{Window, Windows};
    use crate::{Point, Rect};
    use alloc::vec;
    use alloc::vec::Vec;

    const W: u32 = 320;
    const H: u32 = 200;
    const WHITE: u32 = 0x00ff_ffff;
    /// Window-surface key of the back buffer in the GDI model.
    const BACK: u64 = 7;

    struct Back(Vec<u32>);
    impl External for Back {
        fn pixels(&mut self, key: u64) -> Option<&mut [u32]> {
            (key == BACK).then_some(&mut self.0[..])
        }
    }
    /// 6x12 cells whose glyphs are a solid 4x8 block of ink.
    struct Blocks;
    impl TextBackend for Blocks {
        fn metrics(&self, _: &LogFont) -> TextMetrics {
            TextMetrics {
                height: 12,
                ascent: 10,
                descent: 2,
                ave_char_width: 6,
                max_char_width: 6,
                weight: 400,
                ..TextMetrics::default()
            }
        }
        fn advances(&self, _: &LogFont, text: &[u16]) -> Vec<i32> {
            text.iter().map(|_| 6).collect()
        }
        fn draw(
            &self,
            _: &LogFont,
            text: &[u16],
            c: &mut Canvas,
            o: Point,
            color: u32,
            clip: &Region,
        ) -> crate::Result<()> {
            for (i, &u) in text.iter().enumerate() {
                if u == 0x20 {
                    continue;
                }
                let x = o.x + 6 * i as i32 + 1;
                for y in o.y + 2..o.y + 10 {
                    for dx in 0..4 {
                        raster::plot(c, Point::new(x + dx, y), color, 13, clip);
                    }
                }
            }
            Ok(())
        }
    }
    /// The window server's commit rules: the section is captured into the
    /// trusted snapshot (whole or band rows) only while the buffer is
    /// attached, and every successful commit consumes the attachment.
    struct Server {
        section: Vec<u32>,
        snapshot: Vec<u32>,
        attached: bool,
        commits: usize,
    }
    impl Service for Server {
        fn attach(&mut self) -> Result<(), Fail> {
            if self.attached {
                return Err(Fail::WouldBlock);
            }
            self.attached = true;
            Ok(())
        }
        fn commit(&mut self, band: Option<(u32, u32)>) -> Result<(), Fail> {
            if !self.attached {
                return Err(Fail::Other);
            }
            let (t, b) = band.unwrap_or((0, H));
            let rows = t as usize * W as usize..b as usize * W as usize;
            self.snapshot[rows.clone()].copy_from_slice(&self.section[rows]);
            self.attached = false;
            self.commits += 1;
            Ok(())
        }
    }

    /// user32 in miniature: a top-level window with an EDIT child, their
    /// WM_PAINT handlers over surface DCs, and EndPaint's band present.
    struct App {
        windows: Windows,
        gdi: Gdi,
        back: Back,
        server: Server,
        presenter: Presenter,
        main: Handle,
        edit: Handle,
        model: Edit,
    }
    impl App {
        fn new() -> Self {
            let mut windows = Windows::new();
            let mut main = Window::new(0xc000, ws::OVERLAPPEDWINDOW | ws::VISIBLE, 0, 1);
            // Screen rect; the client starts below a 20-pixel menu bar.
            main.rect = Rect::from_size(64, 64, W as i32, H as i32);
            main.client = Rect::new(0, 20, W as i32, H as i32);
            let main = windows.insert(main).unwrap();
            let style = ws::CHILD | ws::VISIBLE | es::MULTILINE;
            let mut edit = Window::new(0xc001, style, 0, 1);
            edit.parent = Some(main);
            edit.rect = Rect::new(0, 0, W as i32, 150);
            // WS_EX_CLIENTEDGE: a 2-pixel frame around the client.
            edit.client = Rect::new(2, 2, W as i32 - 2, 148);
            let edit = windows.insert(edit).unwrap();
            let mut app = Self {
                windows,
                gdi: Gdi::new().unwrap(),
                back: Back(vec![WHITE; (W * H) as usize]),
                server: Server {
                    section: vec![0; (W * H) as usize],
                    snapshot: vec![0; (W * H) as usize],
                    attached: false,
                    commits: 0,
                },
                presenter: Presenter::default(),
                main,
                edit,
                model: Edit::new(style),
            };
            // ShowWindow/UpdateWindow paint before the first configure;
            // CONFIGURED then acknowledges the epoch and presents it all.
            app.windows.invalidate(main, None, true);
            app.pump();
            app.presenter.configured();
            app.presenter.acknowledged = true;
            app.present(None);
            app
        }
        fn present(&mut self, changed: Option<(u32, u32)>) {
            let Some((top, bottom)) = self.presenter.rows(changed, H) else {
                return;
            };
            let rows = top as usize * W as usize..bottom as usize * W as usize;
            self.server.section[rows.clone()].copy_from_slice(&self.back.0[rows]);
            let _ = self
                .presenter
                .commit((top, bottom), changed.is_some(), true, &mut self.server);
        }
        /// Dispatch WM_PAINT until nothing needs painting.
        fn pump(&mut self) {
            while let Some(h) = self.windows.paint_candidate(1, &|_| true) {
                let w = self.windows.get_mut(h).unwrap();
                let update = core::mem::take(&mut w.update);
                w.frame_dirty = false;
                w.erase = false;
                let (cw, ch) = w.client_size();
                // The surface starts at the top-level's screen rectangle.
                let surface = Point::new(64, 64);
                let (origin, visible) = self
                    .windows
                    .surface_target(h, false, surface, Some(&update))
                    .unwrap();
                let rows = visible.bounds();
                let dc = self
                    .gdi
                    .create_surface_dc(BACK, W, H, W as usize, origin, visible)
                    .unwrap();
                let white = self.gdi.stock(stock::WHITE_BRUSH).unwrap();
                self.gdi
                    .fill_rect(dc, Rect::new(0, 0, cw, ch), white, &mut self.back)
                    .unwrap();
                if h == self.edit {
                    self.gdi.state_mut(dc).unwrap().bk_mode = 1;
                    let text = self.model.text().to_vec();
                    for (i, (s, e)) in self.model.lines().into_iter().enumerate() {
                        let at = Point::new(2, 1 + 12 * i as i32);
                        self.gdi
                            .text_out(dc, at, &text[s..e], None, None, &Blocks, &mut self.back)
                            .unwrap();
                    }
                }
                self.gdi.delete_dc(dc).unwrap();
                if rows.top < rows.bottom {
                    self.present(Some((rows.top as u32, rows.bottom as u32)));
                }
            }
        }
        /// WM_CHAR to the edit control: model, then InvalidateRect.
        fn type_text(&mut self, text: &str) {
            for u in text.encode_utf16() {
                let o = self.model.char(u);
                assert!(!o.notify.is_empty(), "the edit control changed");
                self.windows.invalidate(self.edit, None, true);
                self.pump();
            }
        }
        /// The committed pixels of the edit control's client area.
        fn committed_edit(&self) -> Vec<u32> {
            let o = self.windows.client_origin(self.edit);
            let (cw, ch) = self.windows.get(self.edit).unwrap().client_size();
            let (x0, y0) = (o.x - 64, o.y - 64);
            let mut out = Vec::new();
            for y in y0..y0 + ch {
                for x in x0..x0 + cw {
                    out.push(self.server.snapshot[(y * W as i32 + x) as usize]);
                }
            }
            out
        }
    }

    #[test]
    fn typed_edit_text_reaches_the_committed_surface() {
        let mut app = App::new();
        assert_eq!(app.server.commits, 1, "first frame");
        let before = app.committed_edit();
        assert!(before.iter().all(|&p| p == WHITE), "empty edit control");
        app.type_text("Door P13 editor");
        assert_eq!(app.model.len(), 15);
        let after = app.committed_edit();
        let changed = before.iter().zip(&after).filter(|(a, b)| a != b).count();
        let ink = after.iter().filter(|&&p| p == 0).count();
        // 13 non-space glyphs of 4x8 ink, all inside the edit rectangle.
        assert_eq!(ink, 13 * 32, "glyph ink committed");
        assert_eq!(changed, ink);
        assert!(app.server.commits > 15, "one band per WM_PAINT");
        // The ink sits at the edit's client origin (surface 2,22), offset by
        // the control's 2-pixel text margin and the glyph cell.
        let o = app.windows.client_origin(app.edit);
        let first = (o.y - 64 + 3) * W as i32 + (o.x - 64 + 3);
        assert_eq!(app.server.snapshot[first as usize], 0);
        assert_eq!(app.windows.client_origin(app.main), Point::new(64, 84));
    }

    #[test]
    fn commits_reattach_the_consumed_buffer_and_retry_when_busy() {
        let mut p = Presenter::default();
        assert_eq!(p.rows(Some((4, 9)), 20), None, "not acknowledged");
        p.acknowledged = true;
        assert_eq!(
            p.rows(Some((4, 9)), 20),
            Some((0, 20)),
            "first frame is whole"
        );
        let mut s = Server {
            section: vec![0; (W * H) as usize],
            snapshot: vec![0; (W * H) as usize],
            attached: false,
            commits: 0,
        };
        p.commit((0, H), false, true, &mut s).unwrap();
        assert!(p.committed && !p.attached && !s.attached);
        assert_eq!(p.rows(Some((4, 9)), H), Some((4, 9)), "then bands");
        p.commit((4, 9), true, true, &mut s).unwrap();
        assert_eq!(s.commits, 2, "re-attached after the consumed commit");
        // A busy commit keeps the attachment and presents everything next.
        struct Busy(usize);
        impl Service for Busy {
            fn attach(&mut self) -> Result<(), Fail> {
                self.0 += 1;
                Ok(())
            }
            fn commit(&mut self, _: Option<(u32, u32)>) -> Result<(), Fail> {
                Err(Fail::WouldBlock)
            }
        }
        let mut b = Busy(0);
        assert_eq!(p.commit((4, 9), true, true, &mut b), Err(Fail::WouldBlock));
        assert!(p.retry && p.attached);
        assert_eq!(p.rows(Some((4, 9)), H), Some((0, H)));
        let _ = p.commit((0, H), false, true, &mut b);
        assert_eq!(b.0, 1, "the unconsumed attachment is reused");
        p.configured();
        assert_eq!(p, Presenter::default());
    }
}
