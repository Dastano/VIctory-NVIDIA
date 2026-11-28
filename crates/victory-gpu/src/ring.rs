//! Redesign stage 2: the title's ring of constants as memory only the CPU
//! writes (`VICTORY_RING_CPU`, off by default; `1` on, `check` beside the
//! old path).
//!
//! The title keeps its ring of constants inside its 174 MB graphics heap
//! (`crate::port`'s header), which the skinning passes write through a view
//! of all of it (`crate::host::card_steps`' header). So every fill of a
//! constant buffer (`Step::Fill`, `HostList::defer_constants`) reads memory
//! that flights in the air are declared to write: it is planned onto the
//! card (`card_steps::plan`) or waits for them, although no shader ever
//! writes the ring.
//!
//! What is done here:
//!
//! - Learning ([`Phase::Learning`]): the 64 KiB pieces of title memory that
//!   fills read are counted for [`LEARN_FRAMES`] frames; the run of pieces
//!   next to one another (gaps of [`GAP`] pieces at most) that most fills
//!   read is the ring's extent, if it holds a quarter of them at least
//!   ([`extent_of`]). Whole pieces: the compares on the card are by piece.
//! - Confirming ([`Phase::Confirming`], [`CONFIRM_FRAMES`] frames): every
//!   compare that finds bytes the card changed (`Shadow::copy_back_with`,
//!   `relay_merge`, `card_merge`, `card_merge_relay`) tells [`card_wrote`];
//!   bytes inside the extent that no exactly known writer in the air wrote
//!   ([`air_exact`]: copies at their place, copies done on the card, read-
//!   backs, bottom-of-pipe writes) reject the ring for good.
//! - Active ([`Phase::Active`]): with `1`, a fill that lies in the ring and
//!   reads nothing an exactly known writer in the air writes is done on the
//!   CPU at once ([`free`]): no planning on the card, no wait for flights.
//!   With `check`, the old path decides and the new one is counted beside it
//!   (`crate::host`: fills it would serve free, and whether the bytes they
//!   would have read differ from title memory once the flights then in the
//!   air have landed).
//! - A later compare that finds the card wrote inside the ring outside the
//!   exact writers demotes it for good ([`Phase::Demoted`]), logged as an
//!   error: every fill is planned as before from then on.
//!
//! Invariant: the GPU never writes the ring other than through the exact
//! writers (checked by every compare over it); a fill or list reads, for
//! each byte of the ring, title memory as at ExecuteCommandLists: what the
//! title's CPU wrote, which no flight changes.
//!
//! The heap copy's sends to the card leave the ring out ([`hold`]) where
//! nothing of the submission reads it on the card: no refresh step (a view,
//! a copy at its place, a step done on the card) of the submission touches
//! the ring. Those bytes stay in the copy's dirty list and go up with the
//! first submission that does read them; the merges skip them as they skip
//! any title bytes not sent yet. The refreshes of the ring by the heap copy
//! are left alone: a view of the whole heap (the skinning passes') may read
//! any byte of it, so no refresh can be shown to serve fills only.

use std::collections::BTreeMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU8, AtomicU64, Ordering::Relaxed};

/// The pieces the extent is made of (and the card's compares work by).
pub(crate) const PIECE: u64 = 64 << 10;
/// Frames of fills counted before the extent is chosen.
pub(crate) const LEARN_FRAMES: u64 = 120;
/// Frames the extent is watched by the compares before it is used.
pub(crate) const CONFIRM_FRAMES: u64 = 300;
/// Fills counted at least while learning; fewer: learned again.
pub(crate) const MIN_FILLS: u64 = 10_000;
/// Pieces no fill read that a run may hold between two read ones.
pub(crate) const GAP: u64 = 2;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Mode {
    Off,
    On,
    Check,
}

/// `VICTORY_RING_CPU`: `1` on, `check` the old path with the new one
/// counted beside it; else off.
pub(crate) fn mode() -> Mode {
    static MODE: std::sync::OnceLock<Mode> = std::sync::OnceLock::new();
    *MODE.get_or_init(|| match std::env::var("VICTORY_RING_CPU").ok().as_deref().map(str::trim) {
        Some("1") => Mode::On,
        Some("check") => Mode::Check,
        _ => Mode::Off,
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub(crate) enum Phase {
    Learning = 0,
    Confirming = 1,
    Active = 2,
    Rejected = 3,
    Demoted = 4,
}

/// What a step of [`Ring`] did, for the log.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Event {
    /// Learned: the extent (from, to) and the share of fills in it.
    Learned(u64, u64, f64),
    /// Too few fills or no run that holds enough: learned again.
    Relearn,
    Active(u64, u64),
    /// The card wrote (from, to) inside the extent while it was watched.
    Rejected(u64, u64),
    Demoted(u64, u64),
}

/// The ring's state, without the globals (tests make their own).
pub(crate) struct Ring {
    pub(crate) phase: Phase,
    /// Title addresses (from, to), whole pieces; from Confirming on.
    pub(crate) extent: (u64, u64),
    /// The frame the phase began (learning: the first fill counted).
    began: Option<u64>,
    /// Learning: fills by piece number.
    pieces: BTreeMap<u64, u64>,
    total: u64,
}

impl Ring {
    pub(crate) const fn new() -> Ring {
        Ring {
            phase: Phase::Learning,
            extent: (0, 0),
            began: None,
            pieces: BTreeMap::new(),
            total: 0,
        }
    }

    /// Learning: fills read `fills` (title address, bytes) in frame `frame`.
    pub(crate) fn learn(&mut self, frame: u64, fills: &[(usize, usize)]) {
        if self.phase != Phase::Learning || fills.is_empty() {
            return;
        }
        self.began.get_or_insert(frame);
        for &(src, len) in fills {
            let (first, last) = (src as u64 / PIECE, (src + len.max(1) - 1) as u64 / PIECE);
            *self.pieces.entry(first).or_default() += 1;
            if last != first {
                *self.pieces.entry(last).or_default() += 1;
            }
            self.total += 1;
        }
    }

    /// Frame `frame` has begun: the phase moves on if its frames are done.
    pub(crate) fn advance(&mut self, frame: u64) -> Option<Event> {
        let began = self.began?;
        match self.phase {
            Phase::Learning if frame >= began + LEARN_FRAMES => {
                let found = extent_of(&self.pieces, self.total);
                self.pieces.clear();
                self.total = 0;
                match found {
                    Some((from, to, share)) => {
                        self.phase = Phase::Confirming;
                        self.extent = (from, to);
                        self.began = Some(frame);
                        Some(Event::Learned(from, to, share))
                    }
                    None => {
                        self.began = None;
                        Some(Event::Relearn)
                    }
                }
            }
            Phase::Confirming if frame >= began + CONFIRM_FRAMES => {
                self.phase = Phase::Active;
                self.began = Some(frame);
                Some(Event::Active(self.extent.0, self.extent.1))
            }
            _ => None,
        }
    }

    /// A compare found that the card changed title memory `[from, to)`;
    /// `exact` the ranges exactly known writers in the air write (in order,
    /// joined). The first stretch inside the extent that none of them
    /// covers rejects or demotes the ring.
    pub(crate) fn card_wrote(&mut self, from: u64, to: u64, exact: &[(usize, usize)]) -> Option<Event> {
        if !matches!(self.phase, Phase::Confirming | Phase::Active) {
            return None;
        }
        let (a, b) = (from.max(self.extent.0), to.min(self.extent.1));
        if a >= b {
            return None;
        }
        let mut at = a;
        let mut left = None;
        for &(x, y) in exact {
            let (x, y) = (x as u64, y as u64);
            if y <= at {
                continue;
            }
            if x > at {
                left = Some((at, x.min(b)));
                break;
            }
            at = y;
            if at >= b {
                break;
            }
        }
        if left.is_none() && at < b {
            left = Some((at, b));
        }
        let (x, y) = left?;
        Some(if self.phase == Phase::Confirming {
            self.phase = Phase::Rejected;
            Event::Rejected(x, y)
        } else {
            self.phase = Phase::Demoted;
            Event::Demoted(x, y)
        })
    }
}

/// The ring's extent from fills by piece: the run of pieces with gaps of
/// [`GAP`] at most that holds the most fills, if it holds a quarter of
/// `total` and `total` is [`MIN_FILLS`] or more. (from, to, share).
pub(crate) fn extent_of(pieces: &BTreeMap<u64, u64>, total: u64) -> Option<(u64, u64, f64)> {
    if total < MIN_FILLS {
        return None;
    }
    let mut best: Option<(u64, u64, u64)> = None;
    let mut run: Option<(u64, u64, u64)> = None;
    let close = |done: (u64, u64, u64), best: &mut Option<(u64, u64, u64)>| {
        if best.is_none_or(|b| done.2 > b.2) {
            *best = Some(done);
        }
    };
    for (&piece, &n) in pieces {
        run = match run {
            Some((first, last, sum)) if piece <= last + GAP + 1 => Some((first, piece, sum + n)),
            done => {
                if let Some(done) = done {
                    close(done, &mut best);
                }
                Some((piece, piece, n))
            }
        };
    }
    if let Some(done) = run {
        close(done, &mut best);
    }
    let (first, last, sum) = best?;
    // (A fill over two pieces counts in both: the share may pass 1.)
    let share = sum as f64 / total as f64;
    (share >= 0.25).then_some((first * PIECE, (last + 1) * PIECE, share.min(1.0)))
}

static STATE: Mutex<Ring> = Mutex::new(Ring::new());
static PHASE: AtomicU8 = AtomicU8::new(Phase::Learning as u8);
static LO: AtomicU64 = AtomicU64::new(0);
static HI: AtomicU64 = AtomicU64::new(0);
/// The frame [`tick`] looks at the phase again.
static NEXT: AtomicU64 = AtomicU64::new(0);
/// The ranges exactly known writers in the air write ([`air_exact`]).
static EXACT: Mutex<Vec<(usize, usize)>> = Mutex::new(Vec::new());

fn phase() -> Phase {
    match PHASE.load(Relaxed) {
        0 => Phase::Learning,
        1 => Phase::Confirming,
        2 => Phase::Active,
        3 => Phase::Rejected,
        _ => Phase::Demoted,
    }
}

fn publish(ring: &Ring) {
    LO.store(ring.extent.0, Relaxed);
    HI.store(ring.extent.1, Relaxed);
    PHASE.store(ring.phase as u8, Relaxed);
}

fn say(event: Event) {
    match event {
        Event::Learned(from, to, share) => tracing::info!(
            "ring of constants (VICTORY_RING_CPU): learned {from:#x}..{to:#x} ({:.1} MB, {:.0} % of fills), watched by the compares \
             for {CONFIRM_FRAMES} frames",
            (to - from) as f64 / 1e6,
            share * 100.0
        ),
        Event::Relearn => tracing::info!("ring of constants (VICTORY_RING_CPU): no run of pieces holds enough fills, learned again"),
        Event::Active(from, to) => tracing::info!(
            "ring of constants (VICTORY_RING_CPU={}): {from:#x}..{to:#x} confirmed, no compare found the card writing it",
            if mode() == Mode::Check { "check" } else { "1" }
        ),
        Event::Rejected(from, to) => tracing::warn!(
            "ring of constants (VICTORY_RING_CPU): a compare found the card writing {from:#x}..{to:#x} inside the extent while it was \
             watched: not used"
        ),
        Event::Demoted(from, to) => {
            STATS[S_DEMOTED].fetch_add(1, Relaxed);
            tracing::error!(
                "ring of constants (VICTORY_RING_CPU): a compare found the card writing {from:#x}..{to:#x} inside the ring: demoted for \
                 good, every fill planned as before (fills before this may have read bytes a flight then changed)"
            )
        }
    }
}

/// Whether fills are counted for learning.
pub(crate) fn learning() -> bool {
    mode() != Mode::Off && phase() == Phase::Learning
}

/// Learning: fills of a list read `fills` (title address, bytes).
pub(crate) fn learn(fills: &[(usize, usize)]) {
    if fills.is_empty() {
        return;
    }
    let frame = crate::frame::current();
    STATE.lock().unwrap_or_else(|e| e.into_inner()).learn(frame, fills);
}

/// Once a submission: the phase moves on when its frames are done.
pub(crate) fn tick() {
    if mode() == Mode::Off {
        return;
    }
    let frame = crate::frame::current();
    if frame < NEXT.load(Relaxed) {
        return;
    }
    NEXT.store(frame + 1, Relaxed);
    let event = {
        let mut ring = STATE.lock().unwrap_or_else(|e| e.into_inner());
        let event = ring.advance(frame);
        publish(&ring);
        event
    };
    if let Some(event) = event {
        say(event);
    }
}

/// Whether `[src, src + len)` lies in the confirmed ring.
pub(crate) fn contains(src: usize, len: usize) -> bool {
    phase() == Phase::Active && LO.load(Relaxed) <= src as u64 && (src + len) as u64 <= HI.load(Relaxed)
}

/// Whether a fill of `[src, src + len)` may be served free
/// (`VICTORY_RING_CPU=1`, the ring confirmed and holding it); the caller
/// still asks the exact writers in the air.
pub(crate) fn free(src: usize, len: usize) -> bool {
    mode() == Mode::On && contains(src, len)
}

/// Whether the ring is used for decisions ([`free`], the held sends).
pub(crate) fn holds() -> bool {
    mode() == Mode::On && phase() == Phase::Active
}

/// The extent being watched or used, if any.
pub(crate) fn extent() -> Option<(u64, u64)> {
    matches!(phase(), Phase::Confirming | Phase::Active).then(|| (LO.load(Relaxed), HI.load(Relaxed)))
}

/// Whether a compare's bytes `[from, to)` (title addresses) are to be told
/// to [`card_wrote`].
pub(crate) fn watching(from: usize, to: usize) -> bool {
    mode() != Mode::Off && extent().is_some_and(|(lo, hi)| (from as u64) < hi && lo < to as u64)
}

/// A compare found the card changed title memory `[from, to)`.
pub(crate) fn card_wrote(from: usize, to: usize) {
    if from >= to || !watching(from, to) {
        return;
    }
    let exact = EXACT.lock().unwrap_or_else(|e| e.into_inner()).clone();
    let event = {
        let mut ring = STATE.lock().unwrap_or_else(|e| e.into_inner());
        let event = ring.card_wrote(from as u64, to as u64, &exact);
        publish(&ring);
        event
    };
    if let Some(event) = event {
        say(event);
    }
}

/// What exactly known writers in the air write now (in order, joined).
pub(crate) fn air_exact(ranges: &[(usize, usize)]) {
    if mode() == Mode::Off {
        return;
    }
    let mut exact = EXACT.lock().unwrap_or_else(|e| e.into_inner());
    exact.clear();
    exact.extend_from_slice(ranges);
}

/// The ring's offsets in a copy of title memory `[start, start + len)`, if
/// sends hold it back ([`holds`]).
pub(crate) fn offsets_in(start: u64, len: u64) -> Option<(usize, usize)> {
    if !holds() {
        return None;
    }
    let (lo, hi) = (LO.load(Relaxed).max(start), HI.load(Relaxed).min(start + len));
    (lo < hi).then(|| ((lo - start) as usize, (hi - start) as usize))
}

/// `ranges` (offsets, in order) cut into what goes up and what is held
/// back, `ring` the ring's offsets.
pub(crate) fn split_held(ranges: Vec<(usize, usize)>, ring: (usize, usize)) -> (Vec<(usize, usize)>, Vec<(usize, usize)>) {
    let (mut send, mut held) = (Vec::with_capacity(ranges.len()), Vec::new());
    for (a, b) in ranges {
        let (x, y) = (a.max(ring.0), b.min(ring.1));
        if x >= y {
            send.push((a, b));
            continue;
        }
        if a < x {
            send.push((a, x));
        }
        held.push((x, y));
        if y < b {
            send.push((y, b));
        }
    }
    (send, held)
}

/// Since the last frame line (the demotions: since the start).
pub(crate) static STATS: [AtomicU64; 9] = [const { AtomicU64::new(0) }; 9];
/// Fills from the ring done on the CPU while flights' declared writes
/// covered them.
pub(crate) const S_FREE: usize = 0;
/// Fills from the ring that an exact writer in the air kept on the old path.
pub(crate) const S_EXACT: usize = 1;
/// Ring bytes held back from sends at submissions (summed over them), and
/// ring bytes sent.
pub(crate) const S_HELD: usize = 2;
pub(crate) const S_SENT: usize = 3;
/// `check`: fills the new path would serve free, those whose bytes were
/// compared once the flights had landed, those that differed, submissions
/// that the old path waited for or planned and the new would not have.
pub(crate) const S_WOULD: usize = 4;
pub(crate) const S_COMPARED: usize = 5;
pub(crate) const S_DIFFER: usize = 6;
pub(crate) const S_UNBLOCKED: usize = 7;
pub(crate) const S_DEMOTED: usize = 8;

pub(crate) fn count(i: usize, n: u64) {
    if n > 0 {
        STATS[i].fetch_add(n, Relaxed);
    }
}

/// A hash of `len` bytes of title memory at `src`; `None` if not all there.
pub(crate) fn hash(src: usize, len: usize, scratch: &mut Vec<u8>) -> Option<u64> {
    scratch.clear();
    scratch.resize(len, 0);
    if !crate::host::read_into(src as *const u8, scratch) {
        return None;
    }
    Some(hash_bytes(scratch))
}

/// FNV-1a, by 8 bytes.
fn hash_bytes(bytes: &[u8]) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    let mut chunks = bytes.chunks_exact(8);
    for c in &mut chunks {
        let mut w = [0u8; 8];
        w.copy_from_slice(c);
        h = (h ^ u64::from_le_bytes(w)).wrapping_mul(0x100_0000_01b3);
    }
    for &b in chunks.remainder() {
        h = (h ^ u64::from(b)).wrapping_mul(0x100_0000_01b3);
    }
    h
}

/// `check`: fills the new path would have served free at submission
/// `number` (title address, bytes, hash then), compared once it finished.
static CHECKS: Mutex<Vec<(u64, usize, usize, u64)>> = Mutex::new(Vec::new());

/// The most noted at once (the rest are not compared).
const MOST_CHECKS: usize = 200_000;

pub(crate) fn check_note(number: u64, fills: &[(usize, usize, u64)]) {
    if fills.is_empty() {
        return;
    }
    let mut checks = CHECKS.lock().unwrap_or_else(|e| e.into_inner());
    let room = MOST_CHECKS.saturating_sub(checks.len());
    checks.extend(fills.iter().take(room).map(|&(src, len, h)| (number, src, len, h)));
}

/// `check`: submissions up to `finished` have landed: their noted fills'
/// bytes are compared with title memory now.
pub(crate) fn check_landed(finished: u64) {
    let due: Vec<(u64, usize, usize, u64)> = {
        let mut checks = CHECKS.lock().unwrap_or_else(|e| e.into_inner());
        if checks.first().is_none_or(|c| c.0 > finished) {
            return;
        }
        let cut = checks.partition_point(|c| c.0 <= finished);
        checks.drain(..cut).collect()
    };
    let mut scratch = Vec::new();
    let (mut compared, mut differ) = (0u64, 0u64);
    for (_, src, len, was) in due {
        if let Some(now) = hash(src, len, &mut scratch) {
            compared += 1;
            differ += u64::from(now != was);
        }
    }
    count(S_COMPARED, compared);
    count(S_DIFFER, differ);
}

/// For the frame-rate line.
pub(crate) fn log(frames: f64) {
    if mode() == Mode::Off {
        return;
    }
    let n: Vec<f64> = STATS.iter().take(S_DEMOTED).map(|c| c.swap(0, Relaxed) as f64 / frames).collect();
    let (lo, hi) = (LO.load(Relaxed), HI.load(Relaxed));
    let phase = phase();
    let extent = if lo < hi { format!("{lo:#x}..{hi:#x} ({:.1} MB)", (hi - lo) as f64 / 1e6) } else { "not learned".into() };
    let demoted = STATS[S_DEMOTED].load(Relaxed);
    match mode() {
        Mode::Check => tracing::info!(
            "ring of constants per frame (VICTORY_RING_CPU=check): {phase:?} {extent}; {:.0} fills it would serve free, {:.0} kept by \
             exact writers; {:.2} submissions it would not wait for or plan; {:.0} compared after landing, {:.2} differed; {demoted} \
             demotions",
            n[S_WOULD],
            n[S_EXACT],
            n[S_UNBLOCKED],
            n[S_COMPARED],
            n[S_DIFFER],
        ),
        _ => tracing::info!(
            "ring of constants per frame (VICTORY_RING_CPU=1): {phase:?} {extent}; {:.0} fills served free, {:.0} kept by exact writers; \
             sends: {:.2} MB of ring held back (summed over submissions), {:.2} MB of ring sent; {demoted} demotions",
            n[S_FREE],
            n[S_EXACT],
            n[S_HELD] / 1e6,
            n[S_SENT] / 1e6,
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BASE: u64 = 0x3500_0000;

    fn learned(ring: &mut Ring) {
        // 12,000 fills over a ring of 8 pieces (one of them never read),
        // a few hundred elsewhere.
        let base = BASE as usize;
        let mut fills = Vec::new();
        for i in 0..12_000usize {
            let piece = [0, 1, 2, 4, 5, 6, 7][i % 7];
            fills.push((base + piece * PIECE as usize + (i % 100) * 512, 480));
        }
        for i in 0..300usize {
            fills.push((0x4000_0000 + i * 4096, 256));
        }
        ring.learn(10, &fills);
        assert_eq!(ring.advance(10 + LEARN_FRAMES - 1), None);
        let Some(Event::Learned(from, to, share)) = ring.advance(10 + LEARN_FRAMES) else {
            panic!("not learned");
        };
        assert_eq!((from, to), (BASE, BASE + 8 * PIECE));
        assert!(share > 0.9);
        assert_eq!(ring.phase, Phase::Confirming);
    }

    #[test]
    fn the_ring_is_learned_from_fills_and_confirmed() {
        let mut ring = Ring::new();
        learned(&mut ring);
        // Writes outside it, and inside it by an exact writer, are no hit.
        assert_eq!(ring.card_wrote(BASE - 64, BASE, &[]), None);
        assert_eq!(ring.card_wrote(BASE + 100, BASE + 200, &[(BASE as usize, BASE as usize + 4096)]), None);
        let began = 10 + LEARN_FRAMES;
        assert_eq!(ring.advance(began + CONFIRM_FRAMES - 1), None);
        assert_eq!(ring.advance(began + CONFIRM_FRAMES), Some(Event::Active(BASE, BASE + 8 * PIECE)));
        assert_eq!(ring.phase, Phase::Active);
    }

    #[test]
    fn too_few_or_scattered_fills_learn_again() {
        let mut ring = Ring::new();
        ring.learn(0, &[(0x1000, 256); 100]);
        assert_eq!(ring.advance(LEARN_FRAMES), Some(Event::Relearn));
        assert_eq!(ring.phase, Phase::Learning);
        // No run holds a quarter of them.
        let fills: Vec<_> = (0..MIN_FILLS as usize).map(|i| (i * 4 * PIECE as usize, 256)).collect();
        ring.learn(200, &fills);
        assert_eq!(ring.advance(200 + LEARN_FRAMES), Some(Event::Relearn));
    }

    #[test]
    fn a_gpu_write_into_the_ring_demotes_it() {
        let mut ring = Ring::new();
        learned(&mut ring);
        ring.advance(10 + LEARN_FRAMES + CONFIRM_FRAMES);
        assert_eq!(ring.phase, Phase::Active);
        // Partly covered by an exact writer: what is left is a hit.
        let exact = [(BASE as usize + 0x100, BASE as usize + 0x180)];
        assert_eq!(ring.card_wrote(BASE + 0x100, BASE + 0x200, &exact), Some(Event::Demoted(BASE + 0x180, BASE + 0x200)));
        assert_eq!(ring.phase, Phase::Demoted);
        // For good.
        assert_eq!(ring.advance(u64::MAX / 2), None);
        assert_eq!(ring.card_wrote(BASE, BASE + 4, &[]), None);
        assert_eq!(ring.phase, Phase::Demoted);
    }

    #[test]
    fn a_gpu_write_while_confirming_rejects_it() {
        let mut ring = Ring::new();
        learned(&mut ring);
        assert_eq!(
            ring.card_wrote(BASE + 7 * PIECE + 8, BASE + 9 * PIECE, &[]),
            Some(Event::Rejected(BASE + 7 * PIECE + 8, BASE + 8 * PIECE))
        );
        assert_eq!(ring.phase, Phase::Rejected);
        assert_eq!(ring.advance(u64::MAX / 2), None);
    }

    #[test]
    fn sends_are_cut_at_the_ring() {
        let (send, held) = split_held(vec![(0, 10), (20, 50), (60, 70), (90, 120)], (30, 100));
        assert_eq!(send, [(0, 10), (20, 30), (100, 120)]);
        assert_eq!(held, [(30, 50), (60, 70), (90, 100)]);
    }
}

