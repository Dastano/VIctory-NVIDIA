//! Frozen host copies (`VICTORY_FREEZE`, tools/rtx4090/REDESIGN.md stage 1)
//! and the per-copy counters behind them (stage 0).
//!
//! A frame of the boat drive asks some 6,000 refreshes of host copies, and
//! most of them copy nothing: the title has not written the memory in many
//! frames, nor have shaders. Each was a step of its list (`Step::Refresh`),
//! walked four times at submission and looked at in every flush.
//!
//! With the switch a copy that no refresh found changed for
//! `VICTORY_FREEZE_FRAMES` frames (30 by default), and that shaders were never
//! declared to write, is frozen: a list that reads it records no refresh
//! step, only the copy once (`Step::Frozen`, again after each title write the
//! list records). The submission looks at each frozen copy its lists read
//! once, in front of the steps ([`front`]), over the whole span ever asked of
//! it: the same look a refresh that copies nothing makes
//! (`Shadow::nothing_new`: the bits of guarded blocks, or the submission's one
//! asking, which holds the frozen spans too), so every write that look sees
//! (a fault on a guarded block, a kernel write, a landing, our own writes,
//! another view of the page) is seen here. If it is not known to copy
//! nothing the span is refreshed there, and a frozen copy whose refresh
//! copied something is thawed (live again: its lists record refresh steps).
//! Thawed twice within 120 frames: banned, never frozen again. A step that
//! writes title memory during the submission (a title buffer copy, a copy at
//! its place, work of ours) is followed by a refresh of the frozen copies
//! recorded after it, as their refresh steps would have been.
//!
//! What it holds to: every byte a list reads is title memory as of the
//! submission, as before. A frozen copy is never left unlooked at: freezing
//! only changes where the look is made and how often.
//!
//! Deviation from the plan: no page is guarded for freezing. The memory a
//! copy of view memory reads is guarded already (its bits tell the look),
//! and write-watched memory is asked about with the submission's other
//! ranges; guarding write-watched memory against the first write would take
//! frozen spans out of that asking, a later step.

use std::sync::Arc;
use std::sync::atomic::{AtomicU8, AtomicU64, Ordering::Relaxed};

use crate::shadow::Shadow;

const LIVE: u8 = 0;
const FROZEN: u8 = 1;
const BANNED: u8 = 2;

/// Frames within which a second thaw bans the copy.
const BAN_FRAMES: u64 = 120;

/// Whether copies are frozen (`VICTORY_FREEZE=1`; off by default).
pub(crate) fn on() -> bool {
    #[cfg(test)]
    if IN_TEST.get() {
        return true;
    }
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("VICTORY_FREEZE").is_some_and(|v| v == "1"))
}

#[cfg(test)]
thread_local! {
    /// [`on`] for this thread's recordings and submissions.
    pub(crate) static IN_TEST: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// The frames a copy must have gone unchanged to be frozen
/// (`VICTORY_FREEZE_FRAMES`, 30 by default).
fn quiet_frames() -> u64 {
    static N: std::sync::OnceLock<u64> = std::sync::OnceLock::new();
    *N.get_or_init(|| {
        std::env::var("VICTORY_FREEZE_FRAMES").ok().and_then(|v| v.trim().parse::<u64>().ok()).filter(|&n| n > 0).unwrap_or(30)
    })
}

/// The frame now, counted from 1.
#[cfg(not(test))]
fn frame() -> u64 {
    crate::frame::current() + 1
}

#[cfg(test)]
fn frame() -> u64 {
    TEST_FRAME.load(Relaxed) + 1
}

/// The frame in tests (all threads: what refreshes on threads beside note).
#[cfg(test)]
pub(crate) static TEST_FRAME: AtomicU64 = AtomicU64::new(0);

/// Since the last line: copies frozen, thawed, banned; refreshes not recorded
/// as steps for a frozen copy; frozen copies looked at in front of a
/// submission, those found unchanged, the bytes the others' refreshes copied;
/// refreshes after title writes in a submission.
static COUNTS: [AtomicU64; 8] = [const { AtomicU64::new(0) }; 8];
const C_FROZEN: usize = 0;
const C_THAWED: usize = 1;
const C_BANNED: usize = 2;
const C_SKIPPED: usize = 3;
const C_LOOKED: usize = 4;
const C_UNCHANGED: usize = 5;
const C_BYTES: usize = 6;
const C_AFTER: usize = 7;

fn count(what: usize, n: u64) {
    COUNTS[what].fetch_add(n, Relaxed);
}

/// A host copy's freezing state and counters ([`Shadow`]'s `freeze`).
pub(crate) struct State {
    state: AtomicU8,
    /// The span of all refreshes asked of the copy (title addresses: lowest,
    /// highest end; `u64::MAX`, 0 before the first).
    lo: AtomicU64,
    hi: AtomicU64,
    /// The span [`front`] looked at last.
    checked: [AtomicU64; 2],
    /// The submission whose lists last read it frozen.
    seen_in: AtomicU64,
    /// Frames ([`frame`]; 0: never): first asked, last found changed by a
    /// refresh, last found written by shaders, last thawed.
    first: AtomicU64,
    changed: AtomicU64,
    shader: AtomicU64,
    thawed: AtomicU64,
    /// Since the last line of the busiest: refreshes asked, refreshes that
    /// copied something, fills that read the copy's memory.
    asked: AtomicU64,
    copying: AtomicU64,
    fills: AtomicU64,
}

impl Default for State {
    fn default() -> State {
        State {
            state: AtomicU8::new(LIVE),
            lo: AtomicU64::new(u64::MAX),
            hi: AtomicU64::new(0),
            checked: [AtomicU64::new(0), AtomicU64::new(0)],
            seen_in: AtomicU64::new(0),
            first: AtomicU64::new(0),
            changed: AtomicU64::new(0),
            shader: AtomicU64::new(0),
            thawed: AtomicU64::new(0),
            asked: AtomicU64::new(0),
            copying: AtomicU64::new(0),
            fills: AtomicU64::new(0),
        }
    }
}

impl State {
    /// A refresh of `[from, to)` is asked (when a list reads it).
    pub(crate) fn asked(&self, from: usize, to: usize) {
        self.asked.fetch_add(1, Relaxed);
        if from < to {
            self.lo.fetch_min(from as u64, Relaxed);
            self.hi.fetch_max(to as u64, Relaxed);
        }
        if self.first.load(Relaxed) == 0 {
            let _ = self.first.compare_exchange(0, frame(), Relaxed, Relaxed);
        }
    }

    /// A refresh of the copy copied `bytes`.
    pub(crate) fn copied(&self, bytes: usize) {
        if bytes > 0 {
            self.copying.fetch_add(1, Relaxed);
            self.changed.store(frame(), Relaxed);
        }
    }

    /// The compare after a submission found `bytes` written by shaders.
    pub(crate) fn shader_found(&self, bytes: usize) {
        if bytes > 0 {
            self.shader.store(frame(), Relaxed);
        }
    }

    /// A fill read the copy's memory.
    pub(crate) fn filled(&self) {
        self.fills.fetch_add(1, Relaxed);
    }

    /// The span asked of it, if any.
    pub(crate) fn span(&self) -> Option<(usize, usize)> {
        let (lo, hi) = (self.lo.load(Relaxed), self.hi.load(Relaxed));
        (lo < hi).then_some((lo as usize, hi as usize))
    }

    /// Whether it is frozen now.
    pub(crate) fn frozen(&self) -> bool {
        self.state.load(Relaxed) == FROZEN
    }

    /// Whether it is banned.
    #[cfg(test)]
    pub(crate) fn banned(&self) -> bool {
        self.state.load(Relaxed) == BANNED
    }

    /// Shaders are declared to write it: never frozen (again).
    pub(crate) fn never(&self) {
        if self.state.swap(BANNED, Relaxed) == FROZEN {
            count(C_BANNED, 1);
        }
    }

    /// A list reads the copy (its refresh was asked just now, [`State::asked`]):
    /// whether it is frozen, and the list records no refresh step for it.
    /// A live copy is frozen here once it has gone unchanged long enough.
    pub(crate) fn reads_frozen(&self, written: bool, in_place: bool) -> bool {
        if !on() {
            return false;
        }
        match self.state.load(Relaxed) {
            FROZEN if !written => {
                count(C_SKIPPED, 1);
                true
            }
            LIVE if !written && !in_place => {
                let (now, n) = (frame(), quiet_frames());
                let quiet = |at: u64| at == 0 || now.saturating_sub(at) >= n;
                let first = self.first.load(Relaxed);
                let ready = first != 0
                    && now.saturating_sub(first) >= n
                    && quiet(self.changed.load(Relaxed))
                    && quiet(self.shader.load(Relaxed))
                    && quiet(self.thawed.load(Relaxed));
                if ready && self.state.compare_exchange(LIVE, FROZEN, Relaxed, Relaxed).is_ok() {
                    count(C_FROZEN, 1);
                    count(C_SKIPPED, 1);
                    return true;
                }
                false
            }
            _ => false,
        }
    }

    /// Its refresh in front of a submission found it changed: live again,
    /// banned if it was thawed not long ago.
    fn thaw(&self) {
        if self.state.compare_exchange(FROZEN, LIVE, Relaxed, Relaxed).is_err() {
            return;
        }
        count(C_THAWED, 1);
        let now = frame();
        self.changed.store(now, Relaxed);
        let before = self.thawed.swap(now, Relaxed);
        if before != 0 && now.saturating_sub(before) <= BAN_FRAMES && self.state.compare_exchange(LIVE, BANNED, Relaxed, Relaxed).is_ok() {
            count(C_BANNED, 1);
        }
    }
}

/// In front of a submission's steps (after its one asking): each copy its
/// lists read frozen (`copies`, each once) is looked at over the whole span
/// asked of it, and refreshed there by `update` (which returns the bytes
/// copied) unless that look tells it would copy nothing. A frozen copy whose
/// refresh copied something in the span looked at last time is thawed; one
/// thawed or banned since its lists recorded it is refreshed so too.
pub(crate) fn front(copies: &[Arc<Shadow>], submission: u64, mut update: impl FnMut(&Shadow, usize, usize) -> usize) {
    for copy in copies {
        let freeze = &copy.freeze;
        freeze.seen_in.store(submission, Relaxed);
        let Some((from, to)) = freeze.span() else {
            continue;
        };
        count(C_LOOKED, 1);
        // (A span that grew since the last look holds pages never taken:
        // copying them is no thaw.)
        let was = [freeze.checked[0].swap(from as u64, Relaxed), freeze.checked[1].swap(to as u64, Relaxed)];
        let same_span = was == [from as u64, to as u64];
        let frozen = freeze.frozen() && !copy.written();
        if frozen && crate::watch::light() && copy.nothing_new(from, to - from) {
            count(C_UNCHANGED, 1);
            continue;
        }
        let copied = update(copy, from, to);
        count(C_BYTES, copied as u64);
        if copy.written() {
            freeze.never();
        } else if frozen && copied > 0 && same_span {
            freeze.thaw();
        }
    }
}

/// A frozen copy recorded after a step that wrote title memory in this
/// submission is refreshed there: counted.
pub(crate) fn refreshed_after_write() {
    count(C_AFTER, 1);
}

/// The fill counts and fill source ranges (`VICTORY_COPY_STATS=fills`).
pub(crate) fn fills_on() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("VICTORY_COPY_STATS").is_ok_and(|v| v.trim() == "fills"))
}

/// The busiest copies' line, every ten seconds (on by default;
/// `VICTORY_COPY_STATS=0`: none).
fn busiest_on() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("VICTORY_COPY_STATS").is_ok_and(|v| v.trim() != "0") || std::env::var_os("VICTORY_COPY_STATS").is_none())
}

/// Since the last line of them: fills from memory of copies of a megabyte
/// or more, by 64 KiB page of the copy (copy id, page) → (fills, bytes).
static FILL_PAGES: std::sync::Mutex<Option<std::collections::HashMap<(u64, u64), (u64, u64)>>> = std::sync::Mutex::new(None);

/// A fill reads `len` bytes of title memory at `src` (with [`fills_on`]).
pub(crate) fn fill(src: usize, len: usize) {
    let Some(copy) = crate::shadow::peek(src as u64, len as u64) else {
        return;
    };
    copy.freeze.filled();
    let (start, bytes) = copy.span();
    if bytes >= 1 << 20 {
        let page = (src as u64 - start) >> 16;
        let mut pages = FILL_PAGES.lock().unwrap_or_else(|e| e.into_inner());
        let e = pages.get_or_insert_with(Default::default).entry((copy.id(), page)).or_default();
        e.0 += 1;
        e.1 += len as u64;
    }
}

/// The frame line's: the freezing counts per frame, and every ten seconds
/// the 20 copies most refreshed since (`VICTORY_COPY_STATS`).
pub(crate) fn line(frames: f64) {
    let n: Vec<f64> = COUNTS.iter().map(|c| c.swap(0, Relaxed) as f64 / frames).collect();
    if on() {
        tracing::info!(
            "frozen copies per frame (VICTORY_FREEZE=1): {:.1} frozen, {:.2} thawed, {:.2} banned; {:.0} refreshes not recorded \
             as steps; {:.0} frozen copies looked at in front of submissions, {:.0} unchanged, {:.2} MB copied for the others; \
             {:.1} refreshed after title writes",
            n[C_FROZEN],
            n[C_THAWED],
            n[C_BANNED],
            n[C_SKIPPED],
            n[C_LOOKED],
            n[C_UNCHANGED],
            n[C_BYTES] / 1e6,
            n[C_AFTER],
        );
    }
    static LAST: std::sync::Mutex<Option<std::time::Instant>> = std::sync::Mutex::new(None);
    {
        let mut last = LAST.lock().unwrap_or_else(|e| e.into_inner());
        let now = std::time::Instant::now();
        match *last {
            Some(at) if now.duration_since(at) < std::time::Duration::from_secs(10) => return,
            _ => *last = Some(now),
        }
    }
    if busiest_on() {
        busiest();
    }
}

/// Logs the 20 copies most refreshed since the last time, and with
/// [`fills_on`] the pages of big copies fills read most; counters reset.
fn busiest() {
    let now = frame();
    let ago = |at: u64| if at == 0 { "never".to_string() } else { now.saturating_sub(at).to_string() };
    let mut all: Vec<(u64, Arc<Shadow>)> = Vec::new();
    let mut fills = 0u64;
    crate::shadow::each_copy(|copy| {
        let f = &copy.freeze;
        let asked = f.asked.load(Relaxed);
        fills += f.fills.load(Relaxed);
        if asked > 0 || f.fills.load(Relaxed) > 0 {
            all.push((asked, copy.clone()));
        }
    });
    all.sort_unstable_by(|a, b| b.0.cmp(&a.0));
    let mut text = String::new();
    for (asked, copy) in all.iter().take(20) {
        let f = &copy.freeze;
        let (start, len) = copy.span();
        let state = match f.state.load(Relaxed) {
            FROZEN => "frozen",
            BANNED => "banned",
            _ => "live",
        };
        let (lo, hi) = f.span().unwrap_or((0, 0));
        text.push_str(&format!(
            "\n  copy {} at {start:#x} ({:.2} MB, asked {lo:#x}..{hi:#x}): {asked} refreshes asked, {} copied something; frames since \
             changed {}, since shaders wrote {}; {} fills read it; {state}, last read frozen in submission {}{}",
            copy.id(),
            len as f64 / 1e6,
            f.copying.load(Relaxed),
            ago(f.changed.load(Relaxed)),
            ago(f.shader.load(Relaxed)),
            f.fills.load(Relaxed),
            f.seen_in.load(Relaxed),
            if copy.written() { ", written by shaders" } else { "" },
        ));
    }
    for (_, copy) in &all {
        copy.freeze.asked.store(0, Relaxed);
        copy.freeze.copying.store(0, Relaxed);
        copy.freeze.fills.store(0, Relaxed);
    }
    tracing::info!("the 20 host copies most refreshed in the last 10 s (VICTORY_COPY_STATS=0: none):{text}");
    if fills_on() {
        let pages = FILL_PAGES.lock().unwrap_or_else(|e| e.into_inner()).take().unwrap_or_default();
        let mut pages: Vec<((u64, u64), (u64, u64))> = pages.into_iter().collect();
        pages.sort_unstable();
        // Runs of pages one after the other, per copy.
        let mut runs: Vec<(u64, u64, u64, u64, u64)> = Vec::new();
        for ((id, page), (n, bytes)) in pages {
            match runs.last_mut() {
                Some(r) if r.0 == id && r.2 == page => {
                    r.2 = page + 1;
                    r.3 += n;
                    r.4 += bytes;
                }
                _ => runs.push((id, page, page + 1, n, bytes)),
            }
        }
        runs.sort_unstable_by(|a, b| b.3.cmp(&a.3));
        let mut text = String::new();
        for (id, from, to, n, bytes) in runs.iter().take(24) {
            text.push_str(&format!(
                "\n  copy {id}: offsets {:#x}..{:#x}, {n} fills, {:.1} KB",
                from << 16,
                to << 16,
                *bytes as f64 / 1024.0
            ));
        }
        tracing::info!("fills in the last 10 s: {fills} from copies; the busiest runs of 64 KiB pages in copies of 1 MB or more:{text}");
    }
}
