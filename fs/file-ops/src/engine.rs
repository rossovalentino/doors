//! Copy, move and permanent-delete operations as resumable, bounded steps.
//!
//! Each public mutating call commits once: the backend's data changes and
//! the updated record share one durable transaction. A step does at most
//! `STEP_BYTES` of file data and `STEP_ITEMS` namespace mutations, so a
//! request holds the single-threaded owner briefly and one step always fits
//! the journal's working set.
use crate::plan::{FLAG_SAME_FOLDER, FLAG_STARTED, FLAG_TEMP, FOLDER_BYTES, MAX_SOURCES, Plan};
use crate::{
    Decision, Error, MAX_DEPTH, Mode, NAME_BYTES, Name, PATH_BYTES, PathBuf, Policy, RECORD_BYTES,
    Result, Stat, State, Volume, copy_name, file_name, join, numbered_name, parent_of, within,
};

/// File bytes copied per step (16 journal pages).
pub const STEP_BYTES: u64 = 64 * 1024;
/// Namespace mutations per step.
pub const STEP_ITEMS: u32 = 16;
const CHUNK: usize = 4096;

/// What a client shows: counts, the current item and any pending conflict.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Progress {
    pub op: u64,
    pub mode: Mode,
    pub state: State,
    pub items_total: u32,
    pub items_done: u32,
    pub skipped: u32,
    pub failures: u32,
    pub bytes_total: u64,
    pub bytes_done: u64,
    pub last_error: Option<Error>,
    /// (source, existing destination) while `state == Conflict`.
    pub conflict: Option<(Stat, Stat)>,
    pub current: Name,
}

/// Engine scratch: the decoded plan, its record bytes and one data chunk.
/// Owners keep one instance in static storage (about 16 KiB).
pub struct Engine {
    pub plan: Plan,
    /// File bytes per step (`STEP_BYTES`; smaller in tests).
    pub step_bytes: u64,
    record: [u8; RECORD_BYTES],
    chunk: [u8; CHUNK],
}
impl Default for Engine {
    fn default() -> Self {
        Self::new()
    }
}
struct Budget {
    bytes: u64,
    items: u32,
}
impl Budget {
    fn spent(&self) -> bool {
        self.bytes == 0 || self.items == 0
    }
    fn item(&mut self) {
        self.items = self.items.saturating_sub(1);
    }
}
/// Temporary destination file of operation `op` (hidden until published).
fn temp_name(op: u64, out: &mut [u8; 25]) -> &str {
    out[..5].copy_from_slice(b"~$fop");
    for i in 0..16 {
        out[5 + i] = b"0123456789abcdef"[((op >> ((15 - i) * 4)) & 15) as usize];
    }
    out[21..].copy_from_slice(b".tmp");
    core::str::from_utf8(out).unwrap_or("~$fop.tmp")
}
impl Engine {
    pub const fn new() -> Self {
        Self {
            plan: Plan::new(0, 0, Mode::Copy, Policy::Ask, 0),
            step_bytes: STEP_BYTES,
            record: [0; RECORD_BYTES],
            chunk: [0; CHUNK],
        }
    }
    fn save<V: Volume>(&mut self, vol: &mut V) -> Result<()> {
        let len = self.plan.encode(&mut self.record)?;
        vol.save_record(self.plan.op, &self.record[..len])
    }
    /// Load operation `op` owned by `owner` (another user's op is NotFound).
    pub fn load<V: Volume>(&mut self, vol: &mut V, op: u64, owner: u32) -> Result<()> {
        let len = vol
            .load_record(op, &mut self.record)?
            .ok_or(Error::NotFound)?;
        let plan = Plan::decode(&self.record[..len])?;
        if plan.op != op || plan.owner != owner {
            return Err(Error::NotFound);
        }
        self.plan = plan;
        Ok(())
    }
    fn commit_plan<V: Volume>(&mut self, vol: &mut V) -> Result<()> {
        self.save(vol)?;
        vol.commit()
    }
    /// Create operation `op`: nothing changes until the first step.
    /// `dest_root` is ignored for `Mode::Delete`.
    pub fn begin<V: Volume>(
        &mut self,
        vol: &mut V,
        op: u64,
        owner: u32,
        mode: Mode,
        policy: Policy,
        dest_root: &str,
    ) -> Result<()> {
        if op == 0 {
            return Err(Error::Invalid);
        }
        if vol.load_record(op, &mut self.record)?.is_some() {
            return Err(Error::Exists);
        }
        let mut plan = Plan::new(op, owner, mode, policy, vol.now());
        if mode != Mode::Delete {
            if !dest_root.starts_with('/') || dest_root.len() > FOLDER_BYTES {
                return Err(Error::Invalid);
            }
            let stat = vol.stat(dest_root)?;
            if !stat.directory {
                return Err(Error::Invalid);
            }
            plan.dest_root.set(dest_root)?;
        }
        plan.state = State::Paused;
        self.plan = plan;
        // Unstarted (no sources yet): Paused until the first `add`.
        self.commit_plan(vol)
    }
    /// Add one source item. All sources share one source folder.
    pub fn add<V: Volume>(&mut self, vol: &mut V, op: u64, owner: u32, source: &str) -> Result<()> {
        self.load(vol, op, owner)?;
        let plan = &self.plan;
        if plan.items_total != 0
            || plan.flag(FLAG_STARTED)
            || plan.source_count as usize >= MAX_SOURCES
            || !source.starts_with('/')
            || source == "/"
        {
            return Err(Error::Invalid);
        }
        let parent = parent_of(source);
        let name = Name::new(file_name(source))?;
        if parent.len() > FOLDER_BYTES
            || plan.source_count != 0 && plan.source_root.as_str() != parent
            || plan.sources().contains(&name)
        {
            return Err(Error::Invalid);
        }
        if plan.mode != Mode::Delete && within(plan.dest_root.as_str(), source) {
            // "The destination folder is a subfolder of the source folder."
            return Err(Error::Invalid);
        }
        vol.stat(source)?;
        let same = plan.mode != Mode::Delete && plan.dest_root.as_str() == parent;
        let plan = &mut self.plan;
        plan.source_root.set(parent)?;
        plan.sources[plan.source_count as usize] = name;
        plan.source_count += 1;
        plan.set_flag(FLAG_SAME_FOLDER, same);
        plan.state = State::Scanning;
        self.commit_plan(vol)
    }
    pub fn query<V: Volume>(&mut self, vol: &mut V, op: u64, owner: u32) -> Result<Progress> {
        self.load(vol, op, owner)?;
        self.progress(vol)
    }
    /// The next of `owner`'s operations after `after`.
    pub fn next<V: Volume>(
        &mut self,
        vol: &mut V,
        mut after: u64,
        owner: u32,
    ) -> Result<Option<Progress>> {
        while let Some(op) = vol.next_record(after)? {
            after = op;
            match self.load(vol, op, owner) {
                Ok(()) => return self.progress(vol).map(Some),
                Err(Error::NotFound | Error::Corrupt) => continue,
                Err(e) => return Err(e),
            }
        }
        Ok(None)
    }
    pub fn pause<V: Volume>(&mut self, vol: &mut V, op: u64, owner: u32) -> Result<()> {
        self.load(vol, op, owner)?;
        match self.plan.state {
            State::Running | State::Scanning => {
                self.plan.state = State::Paused;
                self.commit_plan(vol)
            }
            State::Paused => Ok(()),
            _ => Err(Error::Invalid),
        }
    }
    pub fn resume<V: Volume>(&mut self, vol: &mut V, op: u64, owner: u32) -> Result<()> {
        self.load(vol, op, owner)?;
        match self.plan.state {
            State::Paused if self.plan.source_count == 0 => Err(Error::Invalid),
            State::Paused => {
                self.plan.state = if self.plan.flag(FLAG_STARTED) {
                    State::Running
                } else {
                    State::Scanning
                };
                self.commit_plan(vol)
            }
            State::Running | State::Scanning | State::Conflict => Ok(()),
            _ => Err(Error::Invalid),
        }
    }
    pub fn resolve<V: Volume>(
        &mut self,
        vol: &mut V,
        op: u64,
        owner: u32,
        decision: Decision,
        all: bool,
    ) -> Result<()> {
        self.load(vol, op, owner)?;
        if self.plan.state != State::Conflict {
            return Err(Error::Invalid);
        }
        self.plan.decision = Some(decision);
        if all {
            self.plan.policy = decision.policy();
        }
        self.plan.state = State::Running;
        self.commit_plan(vol)
    }
    /// Cancel: the partial item is discarded, finished items stay.
    pub fn cancel<V: Volume>(&mut self, vol: &mut V, op: u64, owner: u32) -> Result<Progress> {
        vol.commit()?;
        self.load(vol, op, owner)?;
        if !self.plan.state.finished() {
            self.discard_temp(vol)?;
            self.plan.state = State::Cancelled;
            self.plan.decision = None;
            self.commit_plan(vol)?;
        }
        self.progress(vol)
    }
    /// Forget a finished operation (an unfinished one is cancelled first).
    pub fn dismiss<V: Volume>(&mut self, vol: &mut V, op: u64, owner: u32) -> Result<()> {
        self.load(vol, op, owner)?;
        if !self.plan.state.finished() {
            self.discard_temp(vol)?;
        }
        vol.remove_record(op)?;
        vol.commit()
    }
    /// After a restart nothing drives interrupted operations: started ones
    /// come back Paused (conflicts keep waiting for their decision) and
    /// unstarted ones are dropped. Commits once.
    pub fn recover<V: Volume>(&mut self, vol: &mut V) -> Result<u32> {
        let mut after = 0;
        let mut count = 0;
        while let Some(op) = vol.next_record(after)? {
            after = op;
            let Some(len) = vol.load_record(op, &mut self.record)? else {
                continue;
            };
            let Ok(plan) = Plan::decode(&self.record[..len]) else {
                // A record that fails validation is never replayed; it stays
                // for explicit inspection and is reported as Corrupt.
                continue;
            };
            if !plan.flag(FLAG_STARTED) && !plan.state.finished() {
                // Never started (setup may be incomplete): nothing changed,
                // so the record is dropped and the client asks again.
                vol.remove_record(op)?;
                count += 1;
            } else if matches!(plan.state, State::Running | State::Scanning) {
                self.plan = plan;
                self.plan.state = State::Paused;
                self.save(vol)?;
                count += 1;
            }
        }
        vol.commit()?;
        Ok(count)
    }
    fn discard_temp<V: Volume>(&mut self, vol: &mut V) -> Result<()> {
        if self.plan.flag(FLAG_TEMP) {
            let (_, dst_dir) = self.dirs()?;
            let mut buf = [0; 25];
            let temp = join(dst_dir.as_str(), temp_name(self.plan.op, &mut buf))?;
            match vol.remove(temp.as_str()) {
                Ok(()) | Err(Error::NotFound) => (),
                Err(e) => return Err(e),
            }
            self.plan.set_flag(FLAG_TEMP, false);
            self.plan.file_offset = 0;
        }
        Ok(())
    }
    /// Source and destination folders of the current level.
    fn dirs(&self) -> Result<(PathBuf<PATH_BYTES>, PathBuf<PATH_BYTES>)> {
        let plan = &self.plan;
        let mut src = PathBuf::from(plan.source_root.as_str())?;
        let mut dst = PathBuf::from(plan.dest_root.as_str())?;
        if plan.depth > 0 {
            src.push(plan.relative.as_str())?;
            if plan.mode != Mode::Delete {
                dst.push(plan.top_name.as_str())?;
                if let Some((_, rest)) = plan.relative.as_str().split_once('/') {
                    dst.push(rest)?;
                }
            }
        }
        Ok((src, dst))
    }
    fn progress<V: Volume>(&mut self, vol: &mut V) -> Result<Progress> {
        let p = &self.plan;
        let mut conflict = None;
        if p.state == State::Conflict {
            let (src_dir, dst_dir) = self.dirs()?;
            let src = join(src_dir.as_str(), p.current.as_str())?;
            let dst = join(dst_dir.as_str(), p.current.as_str())?;
            if let (Ok(a), Ok(b)) = (vol.stat(src.as_str()), vol.stat(dst.as_str())) {
                conflict = Some((a, b));
            }
        }
        let p = &self.plan;
        Ok(Progress {
            op: p.op,
            mode: p.mode,
            state: p.state,
            items_total: p.items_total,
            items_done: p.items_done,
            skipped: p.skipped,
            failures: p.failures,
            bytes_total: p.bytes_total,
            bytes_done: p.bytes_done,
            last_error: p.last_error,
            conflict,
            current: p.current,
        })
    }
    fn measure<V: Volume>(vol: &mut V, path: &str) -> Result<(u32, u64)> {
        crate::recycle::measure(vol, path)
    }
    /// Advance by one bounded step and commit it with the record.
    pub fn step<V: Volume>(&mut self, vol: &mut V, op: u64, owner: u32) -> Result<Progress> {
        // Isolate unrelated staging first, so this step's transaction holds
        // only its own changes and always fits the journal.
        vol.commit()?;
        self.load(vol, op, owner)?;
        if self.plan.state == State::Scanning {
            let mut items = 0u32;
            let mut bytes = 0u64;
            for i in 0..self.plan.source_count as usize {
                let path = join(
                    self.plan.source_root.as_str(),
                    self.plan.sources[i].as_str(),
                )?;
                if let Ok((n, b)) = Self::measure(vol, path.as_str()) {
                    items = items.saturating_add(n);
                    bytes = bytes.saturating_add(b);
                }
            }
            self.plan.items_total = items;
            self.plan.bytes_total = bytes;
            self.plan.state = State::Running;
            self.plan.set_flag(FLAG_STARTED, true);
            self.commit_plan(vol)?;
            return self.progress(vol);
        }
        if self.plan.state != State::Running {
            return self.progress(vol);
        }
        let mut budget = Budget {
            bytes: self.step_bytes.max(1),
            items: STEP_ITEMS,
        };
        while self.plan.state == State::Running && !budget.spent() {
            match self.advance(vol, &mut budget) {
                Ok(()) => (),
                Err(e) if e.fatal() => return Err(e),
                Err(e) => {
                    // Stop the operation truthfully: drop the partial item
                    // in this same transaction, keep everything finished.
                    self.discard_temp(vol)?;
                    self.plan.failures = self.plan.failures.saturating_add(1);
                    self.plan.last_error = Some(e);
                    self.plan.state = State::Failed;
                }
            }
        }
        self.commit_plan(vol)?;
        self.progress(vol)
    }
    /// One unit of work at the current position.
    fn advance<V: Volume>(&mut self, vol: &mut V, budget: &mut Budget) -> Result<()> {
        if self.plan.source_index as usize >= self.plan.source_count as usize {
            self.plan.state = State::Done;
            self.plan.current = Name::EMPTY;
            return Ok(());
        }
        let (src_dir, _) = self.dirs()?;
        let (name, stat) = if self.plan.depth == 0 {
            let name = self.plan.sources[self.plan.source_index as usize];
            let path = join(src_dir.as_str(), name.as_str())?;
            match vol.stat(path.as_str()) {
                Ok(stat) => (name, stat),
                Err(Error::NotFound) => {
                    // Vanished since it was selected: report, continue.
                    self.plan.failures = self.plan.failures.saturating_add(1);
                    self.plan.last_error = Some(Error::NotFound);
                    self.finish_top();
                    return Ok(());
                }
                Err(e) => return Err(e),
            }
        } else {
            let depth = self.plan.depth as usize;
            match vol.child(src_dir.as_str(), self.plan.cursors[depth])? {
                Some(entry) => entry,
                None => return self.finish_folder(vol, budget),
            }
        };
        if self.plan.current_object != stat.object {
            // A new item (or the source changed under a partial copy).
            self.discard_temp(vol)?;
            self.plan.current = name;
            self.plan.current_object = stat.object;
            self.plan.target = Name::EMPTY;
        }
        match self.plan.mode {
            Mode::Delete => self.delete_entry(vol, budget, &src_dir, name, stat),
            Mode::Copy | Mode::Move => self.place_entry(vol, budget, &src_dir, name, stat),
        }
    }
    /// The current entry is finished: advance the cursor of its level.
    fn finish_entry(&mut self, object: u64) {
        if self.plan.depth == 0 {
            self.finish_top();
        } else {
            let p = &mut self.plan;
            p.cursors[p.depth as usize] = object;
            p.current_object = 0;
            p.target = Name::EMPTY;
            p.decision = None;
        }
    }
    fn finish_top(&mut self) {
        let p = &mut self.plan;
        p.source_index += 1;
        p.top_name = Name::EMPTY;
        p.target = Name::EMPTY;
        p.current_object = 0;
        p.decision = None;
        p.depth = 0;
        p.relative = PathBuf::new();
        p.cursors = [0; MAX_DEPTH + 1];
    }
    fn count_done(&mut self, items: u32, bytes: u64) {
        self.plan.items_done = self.plan.items_done.saturating_add(items);
        self.plan.bytes_done = self.plan.bytes_done.saturating_add(bytes);
    }
    fn count_skipped(&mut self, items: u32, bytes: u64) {
        self.plan.skipped = self.plan.skipped.saturating_add(items);
        self.count_done(items, bytes);
    }
    fn enter(&mut self, name: Name, target: Name) -> Result<()> {
        let p = &mut self.plan;
        if p.depth as usize >= MAX_DEPTH {
            return Err(Error::Invalid);
        }
        if p.depth == 0 {
            p.top_name = target;
        }
        p.relative.push(name.as_str())?;
        p.depth += 1;
        p.cursors[p.depth as usize] = 0;
        p.current_object = 0;
        p.target = Name::EMPTY;
        p.decision = None;
        Ok(())
    }
    /// A folder's children are all processed.
    fn finish_folder<V: Volume>(&mut self, vol: &mut V, budget: &mut Budget) -> Result<()> {
        let (src_dir, _) = self.dirs()?;
        let object = vol.stat(src_dir.as_str())?.object;
        match self.plan.mode {
            Mode::Delete => {
                match vol.remove(src_dir.as_str()) {
                    Ok(()) => (),
                    Err(e @ (Error::Invalid | Error::Denied | Error::Sharing)) => {
                        self.plan.failures = self.plan.failures.saturating_add(1);
                        self.plan.last_error = Some(e);
                    }
                    Err(e) => return Err(e),
                }
                budget.item();
                self.count_done(1, 0);
            }
            Mode::Move => {
                // Merged folder: remove the emptied source; skipped children
                // keep it (Windows leaves such folders in place).
                match vol.remove(src_dir.as_str()) {
                    Ok(()) | Err(Error::Invalid | Error::Exists) => (),
                    Err(e) => return Err(e),
                }
                budget.item();
            }
            Mode::Copy => (),
        }
        let p = &mut self.plan;
        p.depth -= 1;
        p.relative.pop();
        if p.depth == 0 {
            self.finish_top();
        } else {
            p.cursors[p.depth as usize] = object;
            p.current_object = 0;
        }
        Ok(())
    }
    fn delete_entry<V: Volume>(
        &mut self,
        vol: &mut V,
        budget: &mut Budget,
        src_dir: &PathBuf<PATH_BYTES>,
        name: Name,
        stat: Stat,
    ) -> Result<()> {
        if stat.directory {
            return self.enter(name, name);
        }
        let path = join(src_dir.as_str(), name.as_str())?;
        match vol.remove(path.as_str()) {
            Ok(()) => self.count_done(1, stat.bytes),
            Err(e @ (Error::Denied | Error::Sharing)) => {
                // Per-item failure: keep going with the rest.
                self.plan.failures = self.plan.failures.saturating_add(1);
                self.plan.last_error = Some(e);
                self.count_done(1, stat.bytes);
            }
            Err(e) => return Err(e),
        }
        budget.item();
        self.finish_entry(stat.object);
        Ok(())
    }
    /// Pick the destination name for the current entry, or stop for a
    /// decision. Returns `None` when the entry is skipped.
    fn destination<V: Volume>(
        &mut self,
        vol: &mut V,
        dst_dir: &str,
        name: Name,
        stat: Stat,
    ) -> Result<Option<(Name, bool, Option<Stat>)>> {
        if !self.plan.target.is_empty() {
            let dst = join(dst_dir, self.plan.target.as_str())?;
            let existing = vol.stat(dst.as_str()).ok();
            let replace = self.plan.decision == Some(Decision::Replace);
            return Ok(Some((self.plan.target, replace, existing)));
        }
        let top_same = self.plan.depth == 0 && self.plan.flag(FLAG_SAME_FOLDER);
        if top_same {
            if self.plan.mode == Mode::Move {
                return Ok(None);
            }
            for n in 1..1000 {
                let candidate = copy_name(name.as_str(), stat.directory, n)?;
                let dst = join(dst_dir, candidate.as_str())?;
                if matches!(vol.stat(dst.as_str()), Err(Error::NotFound)) {
                    self.plan.target = candidate;
                    return Ok(Some((candidate, false, None)));
                }
            }
            return Err(Error::Exists);
        }
        let dst = join(dst_dir, name.as_str())?;
        let existing = match vol.stat(dst.as_str()) {
            Ok(existing) => existing,
            Err(Error::NotFound) => {
                self.plan.target = name;
                return Ok(Some((name, false, None)));
            }
            Err(e) => return Err(e),
        };
        if existing.directory && stat.directory {
            // Folders merge; their children raise their own conflicts.
            self.plan.target = name;
            return Ok(Some((name, false, Some(existing))));
        }
        let choice = match (self.plan.decision, self.plan.policy) {
            (Some(d), _) => d,
            (None, Policy::Replace) => Decision::Replace,
            (None, Policy::Skip) => Decision::Skip,
            (None, Policy::KeepBoth) => Decision::KeepBoth,
            (None, Policy::Ask) => {
                self.plan.state = State::Conflict;
                return Ok(Some((name, false, Some(existing))));
            }
        };
        match choice {
            Decision::Skip => Ok(None),
            Decision::Replace if existing.directory != stat.directory => {
                // A file cannot replace a folder (or vice versa).
                self.plan.failures = self.plan.failures.saturating_add(1);
                self.plan.last_error = Some(Error::Exists);
                Ok(None)
            }
            Decision::Replace => {
                self.plan.target = name;
                self.plan.decision = Some(Decision::Replace);
                Ok(Some((name, true, Some(existing))))
            }
            Decision::KeepBoth => {
                for n in 2..1000 {
                    let candidate = numbered_name(name.as_str(), stat.directory, n)?;
                    let dst = join(dst_dir, candidate.as_str())?;
                    if matches!(vol.stat(dst.as_str()), Err(Error::NotFound)) {
                        self.plan.target = candidate;
                        return Ok(Some((candidate, false, None)));
                    }
                }
                Err(Error::Exists)
            }
        }
    }
    fn place_entry<V: Volume>(
        &mut self,
        vol: &mut V,
        budget: &mut Budget,
        src_dir: &PathBuf<PATH_BYTES>,
        name: Name,
        stat: Stat,
    ) -> Result<()> {
        let (_, dst_dir) = self.dirs()?;
        let src = join(src_dir.as_str(), name.as_str())?;
        if self.plan.mode == Mode::Move && self.plan.depth == 0 && self.plan.flag(FLAG_SAME_FOLDER)
        {
            // Moving into the folder it is already in changes nothing.
            let (items, bytes) = Self::measure(vol, src.as_str())?;
            self.count_done(items, bytes);
            self.finish_entry(stat.object);
            return Ok(());
        }
        let Some((target, replace, existing)) =
            self.destination(vol, dst_dir.as_str(), name, stat)?
        else {
            let (items, bytes) = if stat.directory {
                Self::measure(vol, src.as_str())?
            } else {
                (1, stat.bytes)
            };
            self.count_skipped(items, bytes);
            self.finish_entry(stat.object);
            return Ok(());
        };
        if self.plan.state == State::Conflict {
            return Ok(());
        }
        let dst = join(dst_dir.as_str(), target.as_str())?;
        let merge = stat.directory && existing.is_some_and(|e| e.directory);
        if self.plan.mode == Mode::Move && !merge {
            // Same volume: one atomic rename moves the whole subtree.
            let (items, bytes) = if stat.directory {
                Self::measure(vol, src.as_str())?
            } else {
                (1, stat.bytes)
            };
            vol.rename(src.as_str(), dst.as_str(), replace)?;
            budget.item();
            self.count_done(items, bytes);
            self.finish_entry(stat.object);
            return Ok(());
        }
        if stat.directory {
            if !merge {
                vol.mkdir(dst.as_str())?;
                budget.item();
            }
            self.count_done(1, 0);
            return self.enter(name, target);
        }
        self.copy_file(vol, budget, &src, &dst_dir, &dst, replace, stat)
    }
    #[allow(clippy::too_many_arguments)]
    fn copy_file<V: Volume>(
        &mut self,
        vol: &mut V,
        budget: &mut Budget,
        src: &PathBuf<PATH_BYTES>,
        dst_dir: &PathBuf<PATH_BYTES>,
        dst: &PathBuf<PATH_BYTES>,
        replace: bool,
        stat: Stat,
    ) -> Result<()> {
        if stat.bytes > vol.file_limit() {
            self.plan.failures = self.plan.failures.saturating_add(1);
            self.plan.last_error = Some(Error::TooLarge);
            self.count_done(1, stat.bytes);
            self.finish_entry(stat.object);
            return Ok(());
        }
        let mut buf = [0; 25];
        let temp = join(dst_dir.as_str(), temp_name(self.plan.op, &mut buf))?;
        if !self.plan.flag(FLAG_TEMP) {
            let file = match vol.create(temp.as_str()) {
                Ok(file) => file,
                // A stale temp from an interrupted, uncommitted attempt.
                Err(Error::Exists) => vol.open_write(temp.as_str())?,
                Err(e) => return Err(e),
            };
            let result = vol.truncate(&file, 0);
            vol.close(file)?;
            result?;
            vol.set_hidden(temp.as_str(), true)?;
            self.plan.set_flag(FLAG_TEMP, true);
            self.plan.file_offset = 0;
            budget.item();
        }
        let input = vol.open_read(src.as_str())?;
        let output = match vol.open_write(temp.as_str()) {
            Ok(output) => output,
            Err(e) => {
                let _ = vol.close(input);
                return Err(e);
            }
        };
        let result = self.pump(vol, budget, &input, &output);
        let closed = vol.close(output).and(vol.close(input));
        let finished = result?;
        closed?;
        if !finished {
            return Ok(());
        }
        vol.set_hidden(temp.as_str(), false)?;
        vol.rename(temp.as_str(), dst.as_str(), replace)?;
        self.plan.set_flag(FLAG_TEMP, false);
        self.plan.file_offset = 0;
        budget.item();
        self.plan.items_done = self.plan.items_done.saturating_add(1);
        self.finish_entry(stat.object);
        Ok(())
    }
    /// Copy chunks within the byte budget; true at end of file.
    fn pump<V: Volume>(
        &mut self,
        vol: &mut V,
        budget: &mut Budget,
        input: &V::File,
        output: &V::File,
    ) -> Result<bool> {
        // A resumed copy continues at the committed length; any later bytes
        // of an uncommitted attempt are cut off first.
        vol.truncate(output, self.plan.file_offset)?;
        while budget.bytes > 0 {
            let want = CHUNK.min(budget.bytes as usize);
            let n = vol.read(input, self.plan.file_offset, &mut self.chunk[..want])?;
            if n == 0 {
                return Ok(true);
            }
            vol.write(output, self.plan.file_offset, &self.chunk[..n])?;
            self.plan.file_offset += n as u64;
            self.plan.bytes_done = self.plan.bytes_done.saturating_add(n as u64);
            budget.bytes = budget.bytes.saturating_sub(n as u64);
        }
        Ok(false)
    }
}
const _: () = assert!(NAME_BYTES < 256);
