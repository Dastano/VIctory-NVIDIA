//! Steps of a submission that read title memory flights in the air may
//! still write, done on the card in the flights' order instead of on the
//! CPU after a wait for them (`VICTORY_CARD_STEPS`, see [`stage`]).

use std::sync::Arc;

use super::*;
use crate::shadow::Shadow;

/// The stage (`VICTORY_CARD_STEPS`; off by default): `1` constant buffers
/// of descriptor tables (`Step::Fill` with a slot of its own), `2` the
/// title's buffer copies (`Step::Copy`) too, with the steps of the same
/// submission that depend on them. Only with the relay's second stage
/// (`relay_exact`).
///
/// Why: eleven dispatches a frame (the skinning passes, in two early
/// submissions) write through a view of the title's whole 174 MB heap, at
/// places they read out of job records in buffers: no bound of what they
/// write is known before they ran (frame capture replay2: 17af15c4015cb4b6,
/// 55679a0547649d0b, 89d08e561897d4e5, fdd56d97a806bcca and three more,
/// all through one view at offset 5920 of the title's bindless table). The
/// title's ring of constants lies in the same heap. One constant buffer of
/// 480 bytes and one copy of 20 KiB a frame, at a new place of the ring
/// each time, waited for these passes to land: 25-30 ms of a 105 ms frame,
/// the submitting thread standing (run record-1; with `VICTORY_FILL_TITLE=1`
/// in run relaycopy-1 the copy alone took over the wait).
///
/// What is done: a list of ours goes to the card in front of the
/// submission's lists, in the same call. It copies the bytes read from the
/// host copy that the flights write into a buffer of ours, and from there
/// to the destination's host copy (a copy), or the fill's descriptor slot
/// is pointed at that buffer (a fill; the slot is the fill's alone, the
/// root signatures say such descriptors are volatile, and the list that
/// names it has not gone to the card).
///
/// Why that is the value the CPU would have read after the wait. After the
/// wait title memory holds, for every byte read, what the title wrote, or
/// what the last of the flights that wrote it wrote (the landings put it
/// there, in order). The host copy on the card holds the same when our list
/// runs: every flight that may write one of these bytes writes this same
/// host copy ([`holder`]: a write through another host copy, a read-back
/// or a value written behind a flight there, and the old wait is taken),
/// they are all ahead of our list on the queue, and the title's own bytes
/// are brought up to date by a refresh step of our list, which is prepared
/// like any other list's (also under flights' declared writes:
/// `stale_under_flights`, which settles if it cannot). Nothing of this
/// submission writes these bytes earlier than our list: the steps are done
/// in front of all its lists in either case, and among themselves in the
/// order they stand in wherever one depends on another: [`plan`] takes
/// every copy and fill of the submission that writes what a taken one
/// reads or writes, or reads what a taken one writes. Every draw that
/// reads a buffer view of the heap relies on the same refresh and the same
/// order; what is new is only that a constant buffer and a copy do too.
/// Where a page is not mapped the host copy may hold other bytes than the
/// CPU's copy would have left: refused.
///
/// For a copy's destination our list declares a shader write, exactly the
/// bytes copied, after a refresh of them: from there on it is what any
/// shader's write is. The relay compares, and puts what changed into title
/// memory before the title is told anything of this flight (its
/// bottom-of-pipe writes and fences come after, in order); CPU steps of
/// later submissions that read it find it declared and wait, or are done
/// here. Refused: a step of the submission that cannot be taken (a texture,
/// vertices expanded, a fill of memory of another kind) reads the
/// destination behind the copy, or a refresh brings another host copy of it
/// up to date ([`plan`], [`elsewhere`]). While such a copy is in the air a
/// submission that refreshes another host copy of its destination, or
/// reads it in a step that is not checked otherwise, waits for it
/// ([`copied_read`]); with the copy done on the CPU it was there at once.
///
/// What stage 2 does not see: code of ours that reads title memory away
/// from a submission's steps (the pass over textures picked by index, while
/// a list is recorded). With the CPU's copy such a read of the destination
/// found the bytes at once; now it finds them when the flight has landed.
/// The landing tells the guards (`crate::watch::landing`), so a texture
/// made from older bytes is made again. This is why stage 2 is its own.
pub(super) fn stage() -> u32 {
    static STAGE: OnceLock<u32> = OnceLock::new();
    *STAGE.get_or_init(|| match std::env::var("VICTORY_CARD_STEPS").ok().as_deref() {
        Some("1") => 1,
        Some("2") => 2,
        _ => 0,
    })
}

/// Whether a copy at its place in a list ([`place`]) is only in the way of
/// what is done on the card in front of the lists if that stands behind it in
/// them (`VICTORY_PLACED_ORDER=1`; off: wherever it stands), and a
/// submission where it does is cut behind the copy's list ([`cut_for`]). Off:
/// the case did not come up once in runs submit-1 to submit-3, so it has only
/// its tests behind it.
///
/// The fills of constant buffers read the title's ring (7,500 a frame in the
/// opening), and one copy of a few hundred bytes a frame writes into it at its
/// place in a list. The whole submission was refused for it, and stood 40 to
/// 60 ms until the flights had landed (run n19).
fn placed_order() -> bool {
    #[cfg(feature = "diagnostics")]
    if let Some(on) = crate::ab::pick("VICTORY_PLACED_ORDER") {
        return on;
    }
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| crate::switch::on("VICTORY_PLACED_ORDER"))
}

/// Whether a fill behind a copy at its place in a list that reads only bytes the copy writes is
/// done on the card out of the bytes the copy reads (`VICTORY_PLACED_READS=1`; off: the
/// submission is refused for it, [`BEHIND_PLACED`], and waits until the flights have landed).
///
/// In the boat cutscene of the opening this was the one reason submissions were refused for:
/// 0.2 to 0.5 of them a frame, each a wait of about 30 ms for the card, 9 to 14 ms of every
/// frame of the submitting thread (runs v2q and cloud4, 2026-11-27, RTX 4070 SUPER: 37.9 ms a
/// frame where the title asks for 33.3).
fn placed_reads() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| crate::switch::on("VICTORY_PLACED_READS"))
}

/// Whether the fills of a submission that has a copy at its place in a list are done on the
/// card all the same where that gives the bytes waiting gives (`VICTORY_CARD_FILLS`, on by
/// default; `0`: refused, [`BEHIND_PLACED`], and the submission waits for the flights):
///
/// - a fill behind the copy that reads nothing but bytes it writes reads them where the copy
///   takes them from ([`read_behind_placed`], what `VICTORY_PLACED_READS=1` does), if title
///   memory there is all mapped (where it is not, the CPU's fill after the wait reads zeros
///   there: the copy left it alone);
/// - a fill ahead of the copy that reads what it writes is done in front of the lists, which
///   is ahead of the copy as it stood ([`plan_with`]: only if no copy done on the card writes
///   what the copy at its place reads or writes, and nothing taken behind it is in its way);
/// - and then no step left on the CPU behind the copy reads what it writes while flights may
///   write its source or destination ([`read_behind_on_cpu`]): title memory has not had its
///   bytes then (`place::Placed::prepare`), the CPU would read what was there before. Refused,
///   [`PLACED_READER`], and waited for, as without the switch.
///
/// Why the bytes are the same. Waiting, the CPU does the steps in their order once the flights
/// have landed: the copy puts its source's bytes into title memory at its place, a fill behind
/// it reads them there, one ahead of it what was there before. On the card our list runs in
/// front of the submission's lists and behind the flights: it reads what title memory holds
/// once they have landed (the host copies they write, brought up to date by its refresh), the
/// copy's source for one behind it, its destination before the copy for one ahead of it. What
/// the copy itself does at its place, and what the lists read, is the same either way.
///
/// Why: in the diagnostics run r90 (RTX 4090, walking) this was the reason of every wait for a
/// fill of upload memory: one submission in 30 frames or so, 15 to 30 ms each, 0.4-0.8 ms of
/// every frame of the submitting thread, 2 ms in the slow seconds. The fills (0x80-0xf0 bytes)
/// read where flights declare writes: the title's heap the skinning passes write through a view
/// of all of it (0x5ff0000+0xae00000, "a view in a table"), or a few KB a copy at its place of a
/// submission before wrote (0xdb0fc6580+0xbd0); and a copy at its place of their own submission
/// writes what they read (no buffer copy was done on the card in that run).
fn card_fills() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("VICTORY_CARD_FILLS").is_none_or(|v| v != "0"))
}

/// Whether a step behind a copy at its place that flights may write the source or the
/// destination of reads that destination, other than on the card ([`card_fills`]); after
/// [`plan_with`], which refuses those taken (and taken or not, a reader is one).
fn read_behind_on_cpu(does: &[Does], written: &[(usize, usize)]) -> bool {
    does.iter().enumerate().any(|(p, d)| {
        let Does::Placed { dst, src, len } = *d else {
            return false;
        };
        if !touches(written, src, src + len) && !touches(written, dst, dst + len) {
            return false;
        }
        let to = (dst, dst + len);
        does[p + 1..].iter().any(|r| match *r {
            Does::Read { from, to: end, .. } => meet((from, end), to),
            Does::Copy { src, len: n, .. } | Does::Fill { src, len: n, .. } | Does::Placed { src, len: n, .. } => meet((src, src + n), to),
            _ => false,
        })
    })
}

/// Fills of `does` that stand behind a copy at its place, read nothing but bytes it writes and
/// would have to be done on the card (flights write what the copy reads or writes, or what
/// the fill reads): they are made to read the same bytes where the copy takes them from.
/// Returns how many.
///
/// A fill done on the card is done in front of all the lists, the copy at its place in one:
/// the fill would read the destination as it was before the copy. What the copy puts there
/// is its source as the submission finds it (so the CPU does it too, when the submission
/// waits: [`place::Placed`] puts the source's bytes into title memory, the fill reads them
/// there), and that the card holds in front of the lists.
///
/// Left as it is, and refused as before: a fill that reads more than the copy wrote, one
/// whose memory cannot be given bytes on the card, and one with a buffer copy between the
/// copy at its place and itself that writes what either of them reads.
fn read_behind_placed(does: &mut [Does], written: &[(usize, usize)]) -> usize {
    read_behind_placed_where(does, written, &mut |_, _| true)
}

/// [`read_behind_placed`], only for fills whose own bytes `mapped_at` says are all mapped
/// ([`card_fills`]).
fn read_behind_placed_where(does: &mut [Does], written: &[(usize, usize)], mapped_at: &mut dyn FnMut(usize, usize) -> bool) -> usize {
    let placed: Vec<(usize, (usize, usize), usize)> = does
        .iter()
        .enumerate()
        .filter_map(|(i, d)| match *d {
            Does::Placed { dst, src, len } => Some((i, (dst, dst + len), src)),
            _ => None,
        })
        .collect();
    if placed.is_empty() {
        return 0;
    }
    let mut moved = 0;
    for i in 0..does.len() {
        let Does::Fill { src, len, asked, to: true } = does[i] else {
            continue;
        };
        // From the fill back: each copy at its place whose bytes it reads (a copy may read
        // what one ahead of it wrote).
        let (mut at, mut before, mut waits) = (src, i, asked);
        let found = loop {
            let Some(&(p, to, from)) = placed.iter().rev().find(|&&(p, to, _)| p < before && meet(to, (at, at + len))) else {
                break before < i;
            };
            let copies = &does[p + 1..before];
            let between =
                |range: (usize, usize)| copies.iter().any(|d| matches!(*d, Does::Copy { dst, len: n, .. } if meet((dst, dst + n), range)));
            if at < to.0 || at + len > to.1 || between((at, at + len)) {
                break false;
            }
            waits |= touches(written, to.0, to.1) || touches(written, from, from + (to.1 - to.0));
            (at, before) = (from + (at - to.0), p);
            if between((at, at + len)) {
                break false;
            }
        };
        // (Nothing in the air writes any of it: title memory has the copy's bytes, the CPU's
        // fill reads them there.)
        if found && waits && mapped_at(src, src + len) {
            does[i] = Does::Fill { src: at, len, asked: true, to: true };
            moved += 1;
        }
    }
    if moved > 0 {
        DONE[4].fetch_add(moved as u64, Ordering::Relaxed);
    }
    moved
}

/// Why a submission whose fills read where a copy at its place does ([`read_behind_placed`]) is
/// refused all the same.
const PLACED_READER: &str = "a step other than a fill done on the card reads what a copy at its place writes";

/// Whether a step reads the destination of a copy at its place whose source or destination
/// flights may write: title memory does not hold that copy's bytes until they have landed
/// (`place::Placed`), so a texture, expanded vertices, a buffer copy or a fill that reads them
/// there reads what was there before. Without [`read_behind_placed`] the fills behind the copy
/// had the submission refused, and it waited; with them out of the way nothing did.
/// Seen as one character's skin drawn as coloured noise for some seconds in the opening (runs
/// d1 and rel2b of 2026-11-27, not in v5q and v6q with the same switches).
fn placed_read_on_cpu(does: &[Does], written: &[(usize, usize)]) -> bool {
    does.iter().enumerate().any(|(p, d)| {
        let Does::Placed { dst, src, len } = *d else {
            return false;
        };
        if !touches(written, src, src + len) && !touches(written, dst, dst + len) {
            return false;
        }
        let to = (dst, dst + len);
        does.iter().enumerate().any(|(i, r)| match *r {
            Does::Read { from, to: end, .. } => meet((from, end), to),
            Does::Copy { src, len: n, .. } | Does::Fill { src, len: n, .. } => meet((src, src + n), to),
            Does::Placed { src, len: n, .. } => i != p && meet((src, src + n), to),
            _ => false,
        })
    })
}

/// Why [`plan`] refuses for [`place`].
const BEHIND_PLACED: &str = "a copy at its place in a list writes what it reads, or reads or writes what it writes";

/// The bytes of our buffer between reading and writing: what one submission
/// may do this way (each place read once, however many fills take it).
const BETWEEN: u64 = 4 << 20;

/// The most steps of a submission done this way.
const MOST: usize = 12288;

/// Where places begin in the buffer between: a constant buffer view's
/// alignment. What lies from a place's end to the next such boundary is no
/// place's.
const PLACE_ALIGN: u64 = 256;

/// Whether the places read into the buffer between are copied in runs
/// (`VICTORY_HEAVY_CARD_RUNS`, on by default; `0`: one copy for each place,
/// as before): a place that follows the one before it in the same host copy
/// as it follows it in the buffer between (at the next boundary there, with
/// the same gap in both) is copied by the same `CopyBufferRegion`
/// ([`join_reads`]).
///
/// Why: the fills of constant buffers read the title's ring of constants,
/// a new place of it for each draw, one after the other. With 6,000-7,000
/// draws a frame (the crowded common room of the first mission, run r59)
/// that was as many copies recorded into our list, a call into the driver
/// each and as many small copies on the card, and the planning of the steps
/// done on the card took 6-7 ms of the submitting thread a frame (1-2 ms
/// with fewer than 3,000 draws), the part of its work that grew most.
///
/// Why the bytes are the same: every place gets the same bytes from the
/// same place of the same host copy as before, and the reads of a level
/// still come before its writes, the levels in their order. What a joined
/// copy writes besides is the gap after a place up to the next boundary:
/// no place lies there (places begin at boundaries, one after another), no
/// step's bytes are taken from it, and it held whatever an earlier
/// submission left there. Its source lies between two places of the same
/// host copy, inside its buffer.
fn joined_reads() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("VICTORY_HEAVY_CARD_RUNS").is_none_or(|v| v != "0"))
}

/// `reads` (source, host copy by address, offset in the source, offset in
/// the buffer between, bytes), in their order, each read joined to the one
/// before it if it reads the same host copy and follows it in both by the
/// same gap, the one in the buffer between up to that one's next
/// [`PLACE_ALIGN`] boundary ([`joined_reads`]).
fn join_reads<T: Copy>(reads: Vec<(T, usize, u64, u64, u64)>) -> Vec<(T, u64, u64, u64)> {
    let mut out: Vec<(T, usize, u64, u64, u64)> = Vec::with_capacity(reads.len());
    for read in reads {
        let (_, shadow, offset, at, len) = read;
        if let Some(last) = out.last_mut() {
            let (end, source_end) = (last.3 + last.4, last.2 + last.4);
            if last.1 == shadow && at == end.next_multiple_of(PLACE_ALIGN) && offset >= source_end && offset - source_end == at - end {
                last.4 = at + len - last.3;
                continue;
            }
        }
        out.push(read);
    }
    out.into_iter().map(|(source, _, offset, at, len)| (source, offset, at, len)).collect()
}

/// Whether the places read into the buffer between are laid out there by
/// host copy and title address instead of in the order the steps first
/// read them (`VICTORY_CARD_RUNS_SORTED`, on by default; `0`: in that
/// order, as before): the places of a level that read the same host copy
/// near one another ([`RUN_GAP`] bytes at most between them, a multiple of
/// [`PLACE_ALIGN`] apart) lie as far apart in the buffer between as in the
/// host copy, and are read by one `CopyBufferRegion` ([`lay_out`]).
///
/// Why: in the boat drive (run r105) the planning of the steps done on the
/// card cost the submitting thread 3.2 ms a frame in the slow seconds (1.7
/// in the fast ones), the largest part of it recording our list (0.95 ms):
/// some 1,600 places a frame read in as many copies, [`joined_reads`]
/// joining next to none of them (585,397 places in 584,309 copies), as the
/// steps read their places mixed between host copies (a draw's constants
/// from several rings) and not one after another. The slots of the fills
/// (0.65 ms) were no longer the largest part.
///
/// Why the bytes are the same: every place still gets the bytes of the same
/// place of the same host copy at the same level, before that level's
/// writes, at a [`PLACE_ALIGN`] boundary of the buffer between; what a run
/// copies besides lies in the gaps between its places in the host copy
/// (inside its buffer, between two of its places) and goes to the gaps
/// between them in the buffer between, where no place lies and from where
/// no step takes bytes (places that overlap in the host copy overlap the
/// same way, with the same bytes). If the gaps take the buffer between
/// beyond [`BETWEEN`], the places are laid in their order as before.
fn sorted_runs() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("VICTORY_CARD_RUNS_SORTED").is_none_or(|v| v != "0"))
}

/// The most bytes between two places of a run ([`sorted_runs`]): what the
/// card copies besides is cheaper than a copy recorded more.
const RUN_GAP: u64 = 4096;

/// Where `places` (host copy by address, title address, bytes, level) lie
/// in the buffer between ([`sorted_runs`]): their offsets, in their order,
/// the runs that read them (the place whose host copy and address a run
/// begins at, offset in the buffer between, bytes), by level, and the bytes
/// of the buffer between used. With `sorted` off each place is a run of its
/// own, laid in their order.
fn lay_out(places: &[(usize, usize, usize, u32)], sorted: bool) -> (Vec<u64>, Vec<(usize, u64, u64)>, u64) {
    let mut ats = vec![0u64; places.len()];
    let mut runs: Vec<(usize, u64, u64)> = Vec::new();
    let mut end = 0u64;
    let mut order: Vec<usize> = (0..places.len()).collect();
    if sorted {
        order.sort_unstable_by_key(|&i| {
            let (shadow, src, len, level) = places[i];
            (level, shadow, src, std::cmp::Reverse(len))
        });
    }
    // The run open: its first place, where it starts in the buffer between,
    // and its end in the title's addresses.
    let mut open: Option<(usize, u64, u64)> = None;
    for i in order {
        let (shadow, src, len, level) = places[i];
        let (src, len) = (src as u64, len as u64);
        if let Some((first, base, run_end)) = &mut open {
            let (s0, a0, _, l0) = places[*first];
            let a0 = a0 as u64;
            if sorted && s0 == shadow && l0 == level && (src - a0) % PLACE_ALIGN == 0 && src <= *run_end + RUN_GAP {
                ats[i] = *base + (src - a0);
                *run_end = (*run_end).max(src + len);
                runs.last_mut().expect("an open run").2 = *run_end - a0;
                end = end.max(*base + (*run_end - a0));
                continue;
            }
        }
        let base = end.next_multiple_of(PLACE_ALIGN);
        ats[i] = base;
        runs.push((i, base, len));
        end = base + len;
        open = Some((i, base, src + len));
    }
    (ats, runs, end)
}

/// Since the last frame-rate line, the planning of steps done on the card
/// ([`Host::card_steps`]), nanoseconds: choosing them, finding what holds
/// their bytes, recording our list, the fills' slots, the steps taken noted.
static PLAN_PARTS: [std::sync::atomic::AtomicU64; 5] = [const { std::sync::atomic::AtomicU64::new(0) }; 5];

/// Since the last frame-rate line: places read into the buffer between,
/// and the copies that read them.
static PLAN_COPIES: [std::sync::atomic::AtomicU64; 2] = [const { std::sync::atomic::AtomicU64::new(0) }; 2];

/// Counts into [`PLAN_PARTS`].
struct Parts(std::cell::Cell<std::time::Instant>);

impl Parts {
    fn begin() -> Parts {
        Parts(std::cell::Cell::new(std::time::Instant::now()))
    }

    /// What came since the last part was part `i`.
    fn part(&self, i: usize) {
        let now = std::time::Instant::now();
        PLAN_PARTS[i].fetch_add((now - self.0.replace(now)).as_nanos() as u64, Ordering::Relaxed);
    }
}

/// How a fill's bytes get where its list reads them, if the card brings
/// them.
#[derive(Clone)]
pub(super) enum FillTo {
    /// A descriptor slot that shows the fill's memory, and nothing else does
    /// (title heap, slot): it is made to show our buffer instead.
    Slot(usize, u32),
    /// A buffer the card can write, filled from its beginning.
    Buffer(ID3D12Resource),
}

/// Title memory something in the air writes: the host copy it goes through
/// (`None`: none, the bytes reach title memory when the flight lands), from,
/// to, and whether it is the destination of a copy done here.
pub(super) struct Writer {
    shadow: Option<Arc<Shadow>>,
    from: usize,
    to: usize,
    copy: bool,
}

/// What `items` of the line write.
pub(super) fn writers(items: &[Late]) -> Vec<Writer> {
    let mut out = Vec::new();
    for item in items {
        match item {
            Late::Run(flight) => {
                for (shadow, ranges) in &flight.back {
                    out.extend(ranges.iter().map(|&(from, to)| Writer {
                        shadow: Some(shadow.clone()),
                        from,
                        to,
                        copy: false,
                    }));
                }
                for &i in &flight.order {
                    for &(_, at, [row, rows, pitch]) in &flight.lists[i].host().deferred().backs {
                        out.push(Writer {
                            shadow: None,
                            from: at,
                            to: at + rows.saturating_sub(1) * pitch + row,
                            copy: false,
                        });
                    }
                }
                out.extend(flight.own.iter().flat_map(Taken::copied));
            }
            Late::Write(at, _, bytes) => out.push(Writer {
                shadow: None,
                from: *at,
                to: *at + *bytes,
                copy: false,
            }),
            Late::Signal(..) => {}
        }
    }
    out
}

/// The destinations of copies done here that have not landed.
pub(super) fn copies(line: &Line) -> Vec<Writer> {
    if stage() < 2 {
        return Vec::new();
    }
    let flying = line.line.iter().filter_map(|l| match l {
        Late::Run(flight) => flight.own.as_ref(),
        _ => None,
    });
    let landing = line.landing.iter().filter(|w| w.copy).map(|w| Writer {
        shadow: w.shadow.clone(),
        ..*w
    });
    flying.flat_map(Taken::copied).chain(landing).collect()
}

/// Whether `runs` refresh another host copy of a destination in `copies`,
/// or read it in a step that flights are not waited for otherwise
/// (`Step::Host`): that reads title memory, where the copy is not yet.
pub(super) fn copied_read(copies: &[Writer], runs: &[&HostList]) -> bool {
    if copies.is_empty() {
        return false;
    }
    runs.iter().any(|list| {
        list.deferred().copies.iter().any(|step| match step {
            Step::Refresh(shadow, at, len) => copies
                .iter()
                .any(|w| w.from < *at + *len && *at < w.to && w.shadow.as_ref().is_none_or(|s| !Arc::ptr_eq(s, shadow))),
            Step::Host(_, (from, to)) => copies.iter().any(|w| w.from < *to && *from < w.to),
            Step::Plane((at, len), _) => copies.iter().any(|w| w.from < at.saturating_add(*len) && *at < w.to),
            _ => false,
        })
    })
}

/// What a step of a submission does to title memory, as the plan sees it.
/// `asked`: flights in the air stop it from being done on the CPU now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Does {
    Copy { dst: usize, src: usize, len: usize, asked: bool },
    /// `to`: its memory can be given the bytes on the card.
    Fill { src: usize, len: usize, asked: bool, to: bool },
    /// Another read on the CPU (a texture, vertices expanded).
    Read { from: usize, to: usize, asked: bool },
    /// A refresh of a host copy (named by its address).
    Refresh { shadow: usize, from: usize, to: usize },
    /// A copy that its list does on the card ([`place`]).
    Placed { dst: usize, src: usize, len: usize },
    /// Anything, anywhere.
    Run,
}

fn meet(a: (usize, usize), b: (usize, usize)) -> bool {
    a.0 < b.1 && b.0 < a.1
}

/// The bytes `d` reads and writes as a copy or a fill (empty ranges for
/// what it does not).
fn reads_writes(d: &Does) -> Option<((usize, usize), (usize, usize))> {
    match *d {
        Does::Copy { dst, src, len, .. } => Some(((src, src + len), (dst, dst + len))),
        Does::Fill { src, len, .. } => Some(((src, src + len), (0, 0))),
        _ => None,
    }
}

fn touched(ranges: &[(usize, usize)], r: (usize, usize)) -> bool {
    touches(ranges, r.0, r.1)
}

/// The steps of `does` (by place) to be done on the card, in order, each
/// with its level; or why not. All that are asked for, and every copy and
/// fill that is in the way of one that is done there: it writes what that
/// one reads or writes, or reads what it writes, whichever comes first. So
/// what is left on the CPU writes none of their bytes, and reads of them
/// only what a taken copy has not been told to write yet: bytes a step
/// ahead of it reads, in title memory as it is.
///
/// `behind`: for [`BEHIND_PLACED`], the copy at its place and the first step
/// behind it that is in its way.
///
/// Steps of a level are done together, every read before every write; a
/// step that writes what an earlier one reads or writes, or reads what it
/// writes, is of a higher level. Our list does them level by level.
fn plan(does: &[Does], stage: u32, by_order: bool, behind: &mut Option<(usize, usize)>) -> Result<Vec<(usize, u32)>, &'static str> {
    plan_with(does, stage, by_order, false, behind)
}

/// [`plan`]; `reads_ahead` ([`card_fills`]): without `by_order`, a copy at its place that
/// taken steps only read, all of them ahead of it, is not in their way either (done in front
/// of the lists they read what was there before it, as they did on the CPU). One that a taken
/// copy writes the source or the destination of still is.
fn plan_with(
    does: &[Does],
    stage: u32,
    by_order: bool,
    reads_ahead: bool,
    behind: &mut Option<(usize, usize)>,
) -> Result<Vec<(usize, u32)>, &'static str> {
    let takes = |d: &Does| match *d {
        Does::Copy { .. } if stage < 2 => Err("a buffer copy (VICTORY_CARD_STEPS=2 does it on the card)"),
        Does::Fill { to: false, .. } => Err("a fill of memory the card cannot be given the bytes of"),
        _ => Ok(()),
    };
    let mut taken = vec![false; does.len()];
    let (mut read, mut written) = (Vec::new(), Vec::new());
    let mut more = Vec::new();
    for (i, d) in does.iter().enumerate() {
        match *d {
            Does::Run => return Err("a step of ours runs"),
            Does::Read { asked: true, .. } => return Err("a texture reads it"),
            Does::Copy { asked: true, .. } | Does::Fill { asked: true, .. } => more.push(i),
            _ => {}
        }
    }
    while !more.is_empty() {
        // (Joined once for the round: one by one the 2,800 fills of a frame
        // sorted the ranges each time, 16 ms of the submitting thread.)
        let (mut reads, mut writes) = (Vec::new(), Vec::new());
        for i in std::mem::take(&mut more) {
            takes(&does[i])?;
            taken[i] = true;
            if let Some((r, w)) = reads_writes(&does[i]) {
                if w.0 < w.1 && meet(r, w) {
                    return Err("a copy onto itself");
                }
                reads.push(r);
                writes.push(w);
            }
        }
        join_ranges(&mut read, reads);
        if writes.iter().any(|w| w.0 < w.1) {
            join_ranges(&mut written, writes);
        }
        if taken.iter().filter(|t| **t).count() > MOST {
            return Err("too many steps depend on it");
        }
        for (i, d) in does.iter().enumerate() {
            // A fill that cannot be taken is looked at below.
            if taken[i] || matches!(d, Does::Fill { to: false, .. }) {
                continue;
            }
            if let Some((r, w)) = reads_writes(d)
                && (touched(&written, r) || touched(&written, w) || touched(&read, w))
            {
                more.push(i);
            }
        }
    }
    // A copy at its place in a list comes after what is done in front of all
    // of them: as it should after the steps ahead of it, but not after those
    // behind it that read or write its bytes.
    for (p, d) in does.iter().enumerate() {
        if let Does::Placed { dst, src, len } = *d
            && (touched(&read, (dst, dst + len)) || touched(&written, (dst, dst + len)) || touched(&written, (src, src + len)))
        {
            let (to, from) = ((dst, dst + len), (src, src + len));
            let in_the_way = |i: usize| {
                reads_writes(&does[i]).is_some_and(|(r, w)| meet(r, to) || w.0 < w.1 && (meet(w, to) || meet(w, from)))
            };
            let found = (p + 1..does.len()).find(|&i| taken[i] && in_the_way(i));
            *behind = found.map(|i| (p, i));
            let only_read = reads_ahead && !touched(&written, to) && !touched(&written, from);
            if !(by_order || only_read) || found.is_some() {
                return Err(BEHIND_PLACED);
            }
        }
    }
    let order: Vec<usize> = (0..does.len()).filter(|&i| taken[i]).collect();
    // What stays on the CPU and reads what an earlier taken copy writes
    // would read it too early.
    let mut behind: Vec<(usize, usize)> = Vec::new();
    for (i, d) in does.iter().enumerate() {
        if taken[i] {
            if let Some((_, w)) = reads_writes(d)
                && w.0 < w.1
            {
                join_ranges(&mut behind, [w]);
            }
            continue;
        }
        match *d {
            Does::Read { from, to, .. } if touched(&behind, (from, to)) => {
                return Err("a texture or expanded vertices read what it writes");
            }
            Does::Fill { src, len, .. } if touched(&behind, (src, src + len)) => {
                return Err("a fill of memory the card cannot be given the bytes of reads what it writes");
            }
            _ => {}
        }
    }
    let mut out: Vec<(usize, u32)> = Vec::with_capacity(order.len());
    // The copies among them, which alone put others behind them or have to
    // stand behind others.
    let mut copies: Vec<usize> = Vec::new();
    for &i in &order {
        let Some((r, w)) = reads_writes(&does[i]) else {
            continue;
        };
        let writes = w.0 < w.1;
        let mut level = 0;
        // Behind what writes what this reads or writes, and, if this
        // writes, behind what reads it.
        let ahead: Box<dyn Iterator<Item = usize>> =
            if writes { Box::new(0..out.len()) } else { Box::new(copies.iter().copied()) };
        for k in ahead {
            let (j, at): (usize, u32) = out[k];
            let Some((jr, jw)) = reads_writes(&does[j]) else {
                continue;
            };
            if at >= level && (meet(jw, r) || meet(jw, w) || meet(jr, w)) {
                level = at + 1;
            }
        }
        if writes {
            copies.push(out.len());
        }
        out.push((i, level));
    }
    Ok(out)
}

/// Whether a refresh of `does` brings a host copy other than `shadow` up to
/// date in `[from, to)`.
fn elsewhere(does: &[Does], shadow: usize, from: usize, to: usize) -> bool {
    does.iter().any(|d| match *d {
        Does::Refresh { shadow: s, from: a, to: b } => s != shadow && meet((a, b), (from, to)),
        _ => false,
    })
}

/// The host copy that holds `[from, to)` on the card once what `writers`
/// write is there: the one they all write it through, or without any the
/// one shaders would be given.
fn holder(writers: &[Writer], from: usize, to: usize) -> Result<Arc<Shadow>, &'static str> {
    let by = |w: &Writer| w.shadow.as_ref().map_or(0, |s| Arc::as_ptr(s) as usize);
    let found = holding(writers.iter().map(|w| (by(w), w.from, w.to)), from, to)?;
    let found = writers.iter().find_map(|w| w.shadow.as_ref().filter(|s| Arc::as_ptr(s) as usize == found));
    held_in(found.cloned(), from, to)
}

/// What [`holder`] finds among `writers` (host copy by address, `0`: none;
/// from; to), in their order: the host copy that all that write `[from,
/// to)` write it through (`0`: none writes it), or why not.
fn holding(writers: impl Iterator<Item = (usize, usize, usize)>, from: usize, to: usize) -> Result<usize, &'static str> {
    let mut found = 0;
    for (s, ..) in writers.filter(|w| meet((w.1, w.2), (from, to))) {
        match s {
            0 => return Err("a read-back or a value behind a flight is written there"),
            s if found != 0 && s != found => return Err("flights write it through two host copies"),
            s => found = s,
        }
    }
    Ok(found)
}

/// The rest of [`holder`], with the host copy the writers write through.
fn held_in(found: Option<Arc<Shadow>>, from: usize, to: usize) -> Result<Arc<Shadow>, &'static str> {
    let shadow = match found {
        Some(s) => s,
        None => crate::shadow::find(from as u64, (to - from) as u64).ok_or("no host copy of it")?,
    };
    let (start, len) = shadow.span();
    if shadow.in_place() || (from as u64) < start || to as u64 > start + len {
        return Err("no host copy holds all of it");
    }
    Ok(shadow)
}

/// Whether the steps given to the card are planned the quicker way
/// (`VICTORY_CARD_PLAN_FAST`, on by default; `0`: as before): what holds
/// the bytes a step reads is looked up in an index of what flights write
/// ([`Writers`]) instead of a walk over all of it, and the fills' slots are
/// found one list at a time instead of a list's lock per fill. The same
/// steps go to the card, doing the same (`the_index_finds_what_the_walk_finds`):
/// only the time differs. Also: the fills' descriptor slots are made with
/// the heaps looked up once per thread, not once per slot.
///
/// Why: with 7,000-10,000 draws a frame the planning took 3-6 ms of the
/// submitting thread a frame. Flights in the air declare thousands of
/// ranges they write (each a host copy's reference counted once more for
/// the planning), and each of a few hundred places a submission's fills
/// read walked all of them.
fn fast() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("VICTORY_CARD_PLAN_FAST").is_none_or(|v| v != "0"))
}

/// What flights in the air write, as [`writers`] and the line's landing
/// give it, asked by place ([`fast`]): the greatest end over a tree of
/// them by start. Copies done here, added as the planning goes on, are
/// looked at one by one behind them.
#[derive(Default)]
struct Writers {
    /// The host copies written through, by address.
    shadows: ByCopy<Arc<Shadow>>,
    /// Host copy by address (`0`: none), from, to: in [`writers`]' order.
    all: Vec<(usize, usize, usize)>,
    /// Places in `all` of those in the tree, by start, and their starts.
    sorted: Vec<u32>,
    starts: Vec<usize>,
    /// The greatest end under each node (the root at 1, the leaves from
    /// `leaves` on, in `sorted`'s order; ends of 0 beyond).
    tree: Vec<usize>,
    leaves: usize,
    hits: Vec<u32>,
}

impl Writers {
    fn of(line: &Line) -> Writers {
        let mut w = Writers::default();
        for item in &line.line {
            match item {
                Late::Run(flight) => {
                    for (shadow, ranges) in &flight.back {
                        let s = w.shadow(shadow);
                        w.all.extend(ranges.iter().map(|&(from, to)| (s, from, to)));
                    }
                    for &i in &flight.order {
                        for &(_, at, [row, rows, pitch]) in &flight.lists[i].host().deferred().backs {
                            w.all.push((0, at, at + rows.saturating_sub(1) * pitch + row));
                        }
                    }
                    for (shadow, from, to) in flight.own.iter().flat_map(|own| &own.copied) {
                        let s = w.shadow(shadow);
                        w.all.push((s, *from, *to));
                    }
                }
                Late::Write(at, _, bytes) => w.all.push((0, *at, *at + *bytes)),
                Late::Signal(..) => {}
            }
        }
        for l in &line.landing {
            let s = l.shadow.as_ref().map_or(0, |shadow| w.shadow(shadow));
            w.all.push((s, l.from, l.to));
        }
        w.index();
        w
    }

    fn shadow(&mut self, shadow: &Arc<Shadow>) -> usize {
        let at = Arc::as_ptr(shadow) as usize;
        self.shadows.entry(at).or_insert_with(|| shadow.clone());
        at
    }

    /// The tree over all there is now.
    fn index(&mut self) {
        let n = self.all.len();
        self.sorted = (0..n as u32).collect();
        self.sorted.sort_unstable_by_key(|&k| self.all[k as usize].1);
        self.starts = self.sorted.iter().map(|&k| self.all[k as usize].1).collect();
        self.leaves = n.next_power_of_two();
        self.tree = vec![0; 2 * self.leaves];
        for (i, &k) in self.sorted.iter().enumerate() {
            self.tree[self.leaves + i] = self.all[k as usize].2;
        }
        for node in (1..self.leaves).rev() {
            self.tree[node] = self.tree[2 * node].max(self.tree[2 * node + 1]);
        }
    }

    /// A copy done here writes `[from, to)` through `shadow`.
    fn push(&mut self, shadow: &Arc<Shadow>, from: usize, to: usize) {
        let s = self.shadow(shadow);
        self.all.push((s, from, to));
    }

    /// [`holding`] over all of them, in their order.
    fn holding(&mut self, from: usize, to: usize) -> Result<usize, &'static str> {
        self.hits.clear();
        // Those that start before `to` and end after `from`.
        let before = self.starts.partition_point(|&s| s < to);
        fn descend(tree: &[usize], sorted: &[u32], hits: &mut Vec<u32>, node: usize, (lo, hi): (usize, usize), before: usize, from: usize) {
            if lo >= before || tree[node] <= from {
                return;
            }
            if hi - lo == 1 {
                hits.push(sorted[lo]);
                return;
            }
            let mid = (lo + hi) / 2;
            descend(tree, sorted, hits, 2 * node, (lo, mid), before, from);
            descend(tree, sorted, hits, 2 * node + 1, (mid, hi), before, from);
        }
        if before > 0 {
            descend(&self.tree, &self.sorted, &mut self.hits, 1, (0, self.leaves), before, from);
        }
        self.hits.sort_unstable();
        let indexed = self.sorted.len();
        let all = &self.all;
        let those = self.hits.iter().map(|&k| all[k as usize]).chain(all[indexed..].iter().copied());
        holding(those, from, to)
    }

    /// [`holder`] by the index.
    fn holder(&mut self, from: usize, to: usize) -> Result<Arc<Shadow>, &'static str> {
        let found = self.holding(from, to)?;
        held_in(self.shadows.get(&found).cloned(), from, to)
    }
}

/// What the steps of `runs` do, and which step each is (its list by address,
/// its place in it). `written`: what flights may write.
fn does_of(written: &[(usize, usize)], runs: &[&HostList]) -> (Vec<Does>, Vec<(usize, usize)>) {
    let mut does = Vec::new();
    // Which step each is. One list's lock at a time: a submission may
    // name a list twice.
    let mut steps = Vec::new();
    for list in runs {
        for (i, step) in list.deferred().copies.iter().enumerate() {
            does.push(match step {
                &Step::Copy(dst, src, len) => Does::Copy {
                    dst,
                    src,
                    len,
                    asked: copy_blocked(written, dst, src, len).is_some(),
                },
                Step::Fill(_, src, len, to) => Does::Fill {
                    src: *src,
                    len: *len,
                    asked: read_blocked(written, *src, *len),
                    to: to.is_some(),
                },
                Step::Texture(staging) => {
                    let (at, len) = staging.memory();
                    Does::Read {
                        from: at,
                        to: at + len,
                        asked: touches(written, at, at + len),
                    }
                }
                &Step::Plane((at, len), _) => Does::Read {
                    from: at,
                    to: at.saturating_add(len),
                    asked: touches(written, at, at.saturating_add(len)),
                },
                &Step::Host(_, (from, to)) if from < to => Does::Read { from, to, asked: false },
                Step::Refresh(shadow, at, len) => Does::Refresh {
                    shadow: Arc::as_ptr(shadow) as usize,
                    from: *at,
                    to: *at + *len,
                },
                Step::Run(_) => Does::Run,
                Step::Placed(p) => Does::Placed { dst: p.dst, src: p.src, len: p.len },
                Step::GpuWrite(..) | Step::Host(..) => continue,
            });
            steps.push((std::ptr::from_ref::<HostList>(list) as usize, i));
        }
    }
    (does, steps)
}

/// With flights in the air: before which of `runs` the submission is cut so
/// that what reads or writes the bytes of a copy at its place in an earlier
/// list ([`BEHIND_PLACED`]) is done on the card in front of a submission of
/// its own, behind that copy.
pub(super) fn cut_for(runs: &[&HostList]) -> Option<usize> {
    if stage() == 0 || !relay_exact() || !placed_order() || runs.len() < 2 {
        return None;
    }
    // (Most submissions have none.)
    let first = runs.iter().position(|l| l.deferred().copies.iter().any(|s| matches!(s, Step::Placed(_))))?;
    if first + 1 == runs.len() {
        return None;
    }
    let written = line().written.clone();
    blocked_by(&written, runs, false)?;
    let (does, steps) = does_of(&written, runs);
    let mut behind = None;
    if plan(&does, stage(), true, &mut behind) != Err(BEHIND_PLACED) {
        return None;
    }
    let (placed, reader) = behind?;
    let place = |i: usize| runs.iter().position(|l| std::ptr::from_ref::<HostList>(l) as usize == steps[i].0);
    let cut = place(placed)? + 1;
    (place(reader)? >= cut).then_some(cut)
}

/// Whether every page of `[from, to)` can be read (through the kernel:
/// asking Windows walks the page tables, milliseconds in the title's heap).
/// `known`: pages that could be, of the same submission: its fills' places
/// lie eight to a page, and each was read whole to find out.
fn mapped(from: usize, to: usize, known: &mut HashMap<usize, ()>) -> bool {
    const PAGE: usize = 4096;
    (from / PAGE..to.div_ceil(PAGE)).all(|page| {
        known.contains_key(&page) || {
            let there = read_into((page * PAGE).max(from) as *const u8, &mut [0]);
            if there {
                known.insert(page, ());
            }
            there
        }
    })
}

/// A table by addresses and numbers of ours.
type HashMap<K, V> = std::collections::HashMap<K, V, std::hash::BuildHasherDefault<AddressHash>>;

/// Where the bytes of the fill that is `step` (its list by address, its place
/// in it) go.
fn fill_to(runs: &[&HostList], step: (usize, usize)) -> Option<FillTo> {
    let list = runs.iter().find(|l| std::ptr::from_ref::<HostList>(l) as usize == step.0)?;
    match list.deferred().copies.get(step.1)? {
        Step::Fill(_, _, _, to) => to.clone(),
        _ => None,
    }
}

/// [`fill_to`] for each of `chosen` that is a fill (`None` for the
/// others), in their order: a list's lock taken once for the fills of it
/// that stand together (a submission may name a list twice: one at a time).
fn fill_targets(runs: &[&HostList], does: &[Does], steps: &[(usize, usize)], chosen: &[(usize, u32)]) -> Vec<Option<FillTo>> {
    let mut out = Vec::with_capacity(chosen.len());
    let mut open: Option<(usize, Option<std::sync::MutexGuard<'_, Deferred>>)> = None;
    for &(i, _) in chosen {
        if !matches!(does[i], Does::Fill { .. }) {
            out.push(None);
            continue;
        }
        let (list, at) = steps[i];
        if open.as_ref().is_none_or(|o| o.0 != list) {
            drop(open.take());
            let found = runs.iter().find(|l| std::ptr::from_ref::<HostList>(l) as usize == list);
            open = Some((list, found.map(|l| l.deferred())));
        }
        let deferred = open.as_ref().and_then(|o| o.1.as_ref());
        out.push(deferred.and_then(|d| match d.copies.get(at)? {
            Step::Fill(_, _, _, to) => to.clone(),
            _ => None,
        }));
    }
    out
}

/// A step done on the card.
struct Op {
    from: (Arc<Shadow>, usize),
    len: usize,
    /// Its place in the buffer between.
    at: u64,
    level: u32,
    to: To,
}

enum To {
    /// A host copy, and the title address.
    Copy(Arc<Shadow>, usize),
    /// A descriptor slot (title heap, slot).
    Slot(usize, u32),
    /// A buffer's beginning.
    Buffer(ID3D12Resource),
}

/// Records a level of what [`stage`] describes into `list` (open): `reads`
/// are copied into `between` (source, offset in it, offset in `between`,
/// bytes), then `writes` out of it (destination, offset in it, offset in
/// `between`, bytes). `between` is left readable as constants (and must be
/// so, unless this is the `first` level), the others in the common state,
/// as the submission's lists expect them.
fn record(
    first: bool,
    list: &ID3D12GraphicsCommandList,
    between: &ID3D12Resource,
    reads: &[(&ID3D12Resource, u64, u64, u64)],
    writes: &[(&ID3D12Resource, u64, u64, u64)],
) {
    let common = D3D12_RESOURCE_STATE_COMMON;
    let (source, dest) = (D3D12_RESOURCE_STATE_COPY_SOURCE, D3D12_RESOURCE_STATE_COPY_DEST);
    let mut sources: Vec<&ID3D12Resource> = Vec::new();
    let mut dests: Vec<&ID3D12Resource> = Vec::new();
    for (all, list) in [(reads, &mut sources), (writes, &mut dests)] {
        for &(buffer, ..) in all {
            if !list.iter().any(|b| b.as_raw() == buffer.as_raw()) {
                list.push(buffer);
            }
        }
    }
    let both = |b: &ID3D12Resource| dests.iter().any(|d| d.as_raw() == b.as_raw());
    let constants = source | D3D12_RESOURCE_STATE_VERTEX_AND_CONSTANT_BUFFER;
    let mut barriers = vec![transition(between, dest, constants)];
    barriers.extend(sources.iter().map(|b| transition(b, source, if both(b) { dest } else { common })));
    let done: Vec<D3D12_RESOURCE_BARRIER> = dests.iter().map(|b| transition(b, dest, common)).collect();
    // SAFETY: an open list; live buffers, and the callers' ranges lie in
    // them. The buffers start in the common state (the list is the first of
    // its call) and are promoted by their first use.
    unsafe {
        if first {
            // What earlier lists' shaders wrote, first.
            list.ResourceBarrier(&[D3D12_RESOURCE_BARRIER {
                Type: D3D12_RESOURCE_BARRIER_TYPE_UAV,
                Flags: D3D12_RESOURCE_BARRIER_FLAG_NONE,
                Anonymous: D3D12_RESOURCE_BARRIER_0 {
                    UAV: std::mem::ManuallyDrop::new(D3D12_RESOURCE_UAV_BARRIER {
                        pResource: std::mem::ManuallyDrop::new(None),
                    }),
                },
            }]);
        } else {
            list.ResourceBarrier(&[transition(between, constants, dest)]);
        }
        for &(buffer, offset, at, len) in reads {
            list.CopyBufferRegion(between, at, buffer, offset, len);
        }
        list.ResourceBarrier(&barriers);
        for &(buffer, offset, at, len) in writes {
            list.CopyBufferRegion(buffer, offset, between, at, len);
        }
        if !done.is_empty() {
            list.ResourceBarrier(&done);
        }
    }
}

/// The list in front of a submission's, and its buffer ([`BETWEEN`] bytes).
struct Own {
    list: HostList,
    between: ID3D12Resource,
}

// SAFETY: D3D12 objects are free-threaded, and the list is touched by one
// thread at a time (the submitting one until it is sent, then the one that
// lands it, then nobody until it is taken out of `POOL`).
unsafe impl Send for Own {}

/// Those that have landed: closed, their steps dropped.
static POOL: Mutex<Vec<Own>> = Mutex::new(Vec::new());

/// Since the last frame-rate line: fills and copies done on the card, their
/// bytes, submissions that waited instead, refused by a check, and fills that read where a
/// copy at its place ahead of them takes its bytes from ([`read_behind_placed`]).
static DONE: [std::sync::atomic::AtomicU64; 5] = [const { std::sync::atomic::AtomicU64::new(0) }; 5];

/// What [`Host::card_steps`] took off a submission: the list that does it,
/// for the submission to run first, and the steps the CPU leaves out (their
/// list by address, their place in it).
pub(super) struct Taken {
    own: Own,
    steps: std::collections::HashSet<(usize, usize), std::hash::BuildHasherDefault<AddressHash>>,
    copied: Vec<(Arc<Shadow>, usize, usize)>,
}

impl Taken {
    pub(super) fn list(&self) -> &HostList {
        &self.own.list
    }

    /// Whether step `i` of `list` is done by this.
    pub(super) fn takes(&self, list: &HostList, i: usize) -> bool {
        self.steps.contains(&(std::ptr::from_ref(list) as usize, i))
    }

    fn copied(&self) -> impl Iterator<Item = Writer> {
        self.copied.iter().map(|(shadow, from, to)| Writer {
            shadow: Some(shadow.clone()),
            from: *from,
            to: *to,
            copy: true,
        })
    }

    /// The submission has run and landed: the list and its buffer are free.
    pub(super) fn landed(self) {
        // The host copies its steps held go; its commands stay until the
        // next use resets it.
        drop(std::mem::take(&mut *self.own.list.deferred()));
        POOL.lock().unwrap_or_else(|e| e.into_inner()).push(self.own);
    }
}

impl Host {
    /// With flights in the air: if what keeps `runs` from being prepared
    /// now is reads of title memory that can all be done on the card
    /// ([`stage`]), the list that does them. `None` if nothing is in the
    /// way or the old wait is due. Under the submission lock.
    pub(super) fn card_steps(&self, runs: &[&HostList]) -> Option<Taken> {
        if stage() == 0 || !relay_exact() {
            return None;
        }
        let (written, copies) = {
            let line = line();
            if line.written.is_empty() {
                return None;
            }
            (line.written.clone(), copies(&line))
        };
        blocked_by(&written, runs, false)?;
        let taken = if copied_read(&copies, runs) {
            Err("a copy done on the card is read through another host copy")
        } else {
            self.take_steps(&written, runs)
        };
        match taken {
            Ok(taken) => taken,
            Err(why) => {
                DONE[3].fetch_add(1, Ordering::Relaxed);
                count_refusal(why);
                #[cfg(feature = "diagnostics")]
                refused(why);
                let _ = why;
                None
            }
        }
    }

    fn take_steps(&self, written: &[(usize, usize)], runs: &[&HostList]) -> Result<Option<Taken>, &'static str> {
        let mut spent = crate::timeline::span("steps on the card: chosen");
        let timed = Parts::begin();
        let (mut does, steps) = does_of(written, runs);
        let fills = card_fills();
        let by_source = if placed_reads() {
            read_behind_placed(&mut does, written) > 0
        } else if fills && does.iter().any(|d| matches!(d, Does::Placed { .. })) {
            let mut known = HashMap::default();
            read_behind_placed_where(&mut does, written, &mut |from, to| mapped(from, to, &mut known));
            false
        } else {
            false
        };
        if by_source && placed_read_on_cpu(&does, written) {
            return Err(PLACED_READER);
        }
        let chosen = plan_with(&does, stage(), placed_order() || by_source, fills, &mut None)?;
        if fills && !chosen.is_empty() && read_behind_on_cpu(&does, written) {
            return Err(PLACED_READER);
        }
        timed.part(0);
        if chosen.is_empty() {
            return Ok(None);
        }
        spent.next("steps on the card: what holds their bytes");
        let fast = fast();
        let (mut writers, mut index) = {
            let line = line();
            if fast {
                (Vec::new(), Some(Writers::of(&line)))
            } else {
                let mut all = writers(&line.line);
                all.extend(line.landing.iter().map(|w| Writer {
                    shadow: w.shadow.clone(),
                    ..*w
                }));
                (all, None)
            }
        };
        // Where the fills' bytes go, found one list at a time ([`fast`]).
        let mut targets = if fast { fill_targets(runs, &does, &steps, &chosen) } else { Vec::new() };
        let mut ops: Vec<Op> = Vec::with_capacity(chosen.len());
        // The places read (host copy, address, bytes, level): where in the
        // buffer between their bytes are.
        let mut places: HashMap<(usize, usize, usize, u32), u64> = HashMap::default();
        // Some 2,800 fills a frame, of a few hundred places: what holds each
        // is looked for once, as long as no copy is added to the writers.
        let mut held: HashMap<(usize, usize), Arc<Shadow>> = HashMap::default();
        let mut room = HashMap::default();
        let mut end = 0u64;
        // With [`sorted_runs`]: the places in the order first read (and
        // their host copies), `at` their number until they are laid out.
        let sorted = sorted_runs();
        let mut listed: Vec<((usize, usize, usize, u32), Arc<Shadow>)> = Vec::new();
        for (n, &(i, level)) in chosen.iter().enumerate() {
            let Some(((src, _), (dst, _))) = reads_writes(&does[i]) else {
                continue;
            };
            let (Does::Copy { len, .. } | Does::Fill { len, .. }) = does[i] else {
                continue;
            };
            if len == 0 {
                return Err("nothing to read");
            }
            let from = match held.get(&(src, len)) {
                Some(from) => from.clone(),
                None => {
                    let from = match &mut index {
                        Some(index) => index.holder(src, src + len)?,
                        None => holder(&writers, src, src + len)?,
                    };
                    held.insert((src, len), from.clone());
                    from
                }
            };
            let place = (Arc::as_ptr(&from) as usize, src, len, level);
            let at = match places.get(&place) {
                Some(&at) => at,
                None if sorted => {
                    if !mapped(src, src + len, &mut room) {
                        return Err("not all of what it reads is mapped");
                    }
                    let at = listed.len() as u64;
                    listed.push((place, from.clone()));
                    places.insert(place, at);
                    at
                }
                None => {
                    if end + len as u64 > BETWEEN {
                        return Err("too much at once");
                    }
                    if !mapped(src, src + len, &mut room) {
                        return Err("not all of what it reads is mapped");
                    }
                    let at = end;
                    end = (end + len as u64).next_multiple_of(PLACE_ALIGN);
                    places.insert(place, at);
                    at
                }
            };
            let to = match does[i] {
                Does::Copy { .. } => {
                    let to = match &mut index {
                        Some(index) => index.holder(dst, dst + len)?,
                        None => holder(&writers, dst, dst + len)?,
                    };
                    if elsewhere(&does, Arc::as_ptr(&to) as usize, dst, dst + len) {
                        return Err("another host copy of what it writes is read");
                    }
                    if !mapped(dst, dst + len, &mut room) {
                        return Err("not all of what it writes is mapped");
                    }
                    // What the steps behind it read of it is in that host
                    // copy, or nowhere.
                    match &mut index {
                        Some(index) => index.push(&to, dst, dst + len),
                        None => writers.push(Writer {
                            shadow: Some(to.clone()),
                            from: dst,
                            to: dst + len,
                            copy: true,
                        }),
                    }
                    held.clear();
                    To::Copy(to, dst)
                }
                _ => match if fast { targets[n].take() } else { fill_to(runs, steps[i]) } {
                    Some(FillTo::Slot(heap, index)) => To::Slot(heap, index),
                    Some(FillTo::Buffer(buffer)) => To::Buffer(buffer),
                    None => continue,
                },
            };
            ops.push(Op {
                from: (from, src),
                len,
                at,
                level,
                to,
            });
        }
        // The runs that read the places ([`lay_out`]), with [`sorted_runs`].
        let mut runs_read: Vec<(u32, usize, u64, u64)> = Vec::new();
        if sorted {
            let keys: Vec<_> = listed.iter().map(|(place, _)| *place).collect();
            let (mut ats, mut runs, mut used) = lay_out(&keys, true);
            if used > BETWEEN {
                (ats, runs, used) = lay_out(&keys, false);
            }
            if used > BETWEEN {
                return Err("too much at once");
            }
            for op in &mut ops {
                op.at = ats[op.at as usize];
            }
            runs_read = runs.into_iter().map(|(first, at, len)| (keys[first].3, first, at, len)).collect();
            // (What the note of the list tells, with diagnostics.)
            #[cfg(feature = "diagnostics")]
            {
                end = used;
            }
            let _ = used;
        }
        timed.part(1);
        spent.next("steps on the card: the list");
        let Some(own) = self.own_list() else {
            return Err("no host list or buffer for it");
        };
        // First every refresh, then the writes: the bytes compared with are
        // the title's. (One step for each host copy, over all that is read of
        // it: a flush brings a copy up to date from the first range asked for
        // to the last, whatever the steps.)
        let mut spans: ByCopy<(Arc<Shadow>, usize, usize)> = ByCopy::default();
        for op in &ops {
            let to = match &op.to {
                To::Copy(to, dst) => Some((to, *dst)),
                _ => None,
            };
            for (shadow, at) in std::iter::once((&op.from.0, op.from.1)).chain(to) {
                let span = spans.entry(Arc::as_ptr(shadow) as usize).or_insert_with(|| (shadow.clone(), at, at + op.len));
                (span.1, span.2) = (span.1.min(at), span.2.max(at + op.len));
            }
        }
        for (shadow, from, to) in spans.into_values() {
            shadow.refresh(&own.list, from as u64, (to - from) as u64);
        }
        let mut copied = Vec::new();
        for op in &ops {
            if let To::Copy(to, dst) = &op.to {
                to.gpu_writes(&own.list, *dst as u64, op.len as u64);
                copied.push((to.clone(), *dst, *dst + op.len));
            }
        }
        #[cfg(feature = "diagnostics")]
        own.list.note_op(|| format!("{} copies of title memory done on the card, {end} bytes (VICTORY_CARD_STEPS)", ops.len()));
        let levels = ops.iter().map(|op| op.level).max().unwrap_or(0);
        let joined = joined_reads();
        let (mut read_places, mut read_copies) = (0u64, 0u64);
        for level in 0..=levels {
            let mut reads = Vec::new();
            let mut writes = Vec::new();
            for &(_, first, at, len) in runs_read.iter().filter(|run| run.0 == level) {
                let ((shadow, src, ..), from) = &listed[first];
                reads.push((&from.buffer, *shadow, from.offset(*src as u64), at, len));
            }
            for op in ops.iter().filter(|op| op.level == level) {
                let len = op.len as u64;
                let shadow = Arc::as_ptr(&op.from.0) as usize;
                let read = (&op.from.0.buffer, shadow, op.from.0.offset(op.from.1 as u64), op.at, len);
                // (Each place once.)
                if !sorted && places.remove(&(shadow, op.from.1, op.len, level)).is_some() {
                    reads.push(read);
                }
                match &op.to {
                    To::Copy(to, dst) => writes.push((&to.buffer, to.offset(*dst as u64), op.at, len)),
                    To::Buffer(buffer) => writes.push((buffer, 0, op.at, len)),
                    To::Slot(..) => {}
                }
            }
            read_places += if sorted { listed.iter().filter(|(place, _)| place.3 == level).count() } else { reads.len() } as u64;
            let reads = if joined {
                join_reads(reads)
            } else {
                reads.into_iter().map(|(buffer, _, offset, at, len)| (buffer, offset, at, len)).collect()
            };
            read_copies += reads.len() as u64;
            record(level == 0, &own.list.list, &own.between, &reads, &writes);
        }
        PLAN_COPIES[0].fetch_add(read_places, Ordering::Relaxed);
        PLAN_COPIES[1].fetch_add(read_copies, Ordering::Relaxed);
        own.list.close();
        // SAFETY: a live buffer.
        let gpu = unsafe { own.between.GetGPUVirtualAddress() };
        timed.part(2);
        spent.next("steps on the card: the slots");
        // (11,000 slots a frame in a street, two descriptors each: on
        // several threads, a slot always on the same one, in the steps'
        // order.)
        let parts = if ops.len() < 256 { 1 } else { 8 };
        crate::land::each_under(parts, &|part| {
            let mine = ops.iter().filter_map(|op| match &op.to {
                To::Slot(heap, index) if *index as usize / 4 % parts == part => Some((*heap, *index, gpu + op.at, op.len as u32)),
                _ => None,
            });
            // (A heap that is gone: nothing can read the slot.)
            if fast {
                crate::descriptor::constant_slots(mine, &own.between);
            } else {
                mine.for_each(|(heap, index, at, len)| crate::descriptor::constant_slot(heap, index, &own.between, at, len));
            }
        });
        timed.part(3);
        let copies = ops.iter().filter(|op| matches!(op.to, To::Copy(..))).count();
        DONE[0].fetch_add((ops.len() - copies) as u64, Ordering::Relaxed);
        DONE[1].fetch_add(copies as u64, Ordering::Relaxed);
        DONE[2].fetch_add(ops.iter().map(|op| op.len as u64).sum(), Ordering::Relaxed);
        #[cfg(feature = "diagnostics")]
        noted(&ops);
        spent.end();
        let taken = Taken {
            own,
            steps: chosen.iter().map(|&(i, _)| steps[i]).collect(),
            copied,
        };
        timed.part(4);
        Ok(Some(taken))
    }

    /// A list of ours open for recording, and its buffer.
    fn own_list(&self) -> Option<Own> {
        let pooled = POOL.lock().unwrap_or_else(|e| e.into_inner()).pop();
        if let Some(own) = pooled {
            own.list.reset();
            return Some(own);
        }
        let between = self.uav_buffer(BETWEEN)?;
        name(&between, || "copies of title memory on the card".into());
        let list = self.new_list().ok()?;
        Some(Own { list, between })
    }
}

/// Diagnostics: the copies and fills done on the card since the last
/// frame-rate line, by bytes, host copy read and host copy written (start
/// and length; zeros for a fill): how many, and the last title addresses.
#[cfg(feature = "diagnostics")]
type Noted = std::collections::HashMap<(usize, (u64, u64), (u64, u64)), (u64, usize, usize)>;
#[cfg(feature = "diagnostics")]
static NOTED: Mutex<Option<Noted>> = Mutex::new(None);

#[cfg(feature = "diagnostics")]
fn noted(ops: &[Op]) {
    let mut noted = NOTED.lock().unwrap_or_else(|e| e.into_inner());
    let noted = noted.get_or_insert_with(Default::default);
    // (Steps of the same kind one after the other counted together: the
    // table is asked once for them.)
    let mut run: Option<((usize, (u64, u64), (u64, u64)), (u64, usize, usize))> = None;
    for op in ops {
        let (to, dst) = match &op.to {
            To::Copy(to, dst) => (to.span(), *dst),
            To::Slot(..) | To::Buffer(_) => ((0, 0), 0),
        };
        let key = (op.len, op.from.0.span(), to);
        match &mut run {
            Some((k, e)) if *k == key => *e = (e.0 + 1, op.from.1, dst),
            _ => {
                if let Some((k, (n, src, dst))) = run.take() {
                    let e = noted.entry(k).or_default();
                    *e = (e.0 + n, src, dst);
                }
                run = Some((key, (1, op.from.1, dst)));
            }
        }
    }
    if let Some((k, (n, src, dst))) = run {
        let e = noted.entry(k).or_default();
        *e = (e.0 + n, src, dst);
    }
}

/// Why submissions were refused since the last line, and how often (few a frame: a list is enough).
static REFUSALS: std::sync::Mutex<Vec<(&'static str, u32)>> = std::sync::Mutex::new(Vec::new());

fn count_refusal(why: &'static str) {
    let mut all = REFUSALS.lock().unwrap_or_else(|e| e.into_inner());
    match all.iter_mut().find(|(w, _)| std::ptr::eq(*w, why)) {
        Some((_, n)) => *n += 1,
        None => all.push((why, 1)),
    }
}

/// For the frame-rate line.
pub(super) fn log(frames: f64) {
    if stage() == 0 {
        return;
    }
    let refusals = std::mem::take(&mut *REFUSALS.lock().unwrap_or_else(|e| e.into_inner()));
    if !refusals.is_empty() {
        let all: Vec<String> = refusals.iter().map(|(why, n)| format!("{:.2} {why}", f64::from(*n) / frames)).collect();
        tracing::info!("submissions refused for the card's steps, per frame: {}", all.join("; "));
    }
    let d: Vec<f64> = DONE.iter().map(|d| d.swap(0, Ordering::Relaxed) as f64 / frames).collect();
    tracing::info!(
        "steps done on the card instead of waiting for flights, per frame (VICTORY_CARD_STEPS={}): {:.2} fills, {:.2} buffer \
         copies, {:.1} KB; {:.2} submissions refused and waited; {:.2} fills read where a copy at its place takes its bytes \n         from (VICTORY_PLACED_READS, VICTORY_CARD_FILLS)",
        stage(),
        d[0],
        d[1],
        d[2] / 1024.0,
        d[3],
        d[4],
    );
    let ms: [f64; 5] = std::array::from_fn(|i| PLAN_PARTS[i].swap(0, Ordering::Relaxed) as f64 / 1e6 / frames);
    let copies: [f64; 2] = std::array::from_fn(|i| PLAN_COPIES[i].swap(0, Ordering::Relaxed) as f64 / frames);
    if ms.iter().sum::<f64>() > 0.0 {
        tracing::info!(
            "steps done on the card, their planning per frame, the submitting thread: {:.2} ms choosing them, {:.2} finding \
             what holds their bytes, {:.2} recording our list ({:.0} places read into the buffer between in {:.0} copies, \
             VICTORY_HEAVY_CARD_RUNS is {}), {:.2} making the fills' slots, {:.2} noting what was taken",
            ms[0],
            ms[1],
            ms[2],
            copies[0],
            copies[1],
            if joined_reads() { "on" } else { "off" },
            ms[3],
            ms[4],
        );
    }
    #[cfg(feature = "diagnostics")]
    {
        let noted = NOTED.lock().unwrap_or_else(|e| e.into_inner()).take().unwrap_or_default();
        let mut noted: Vec<_> = noted.into_iter().collect();
        noted.sort_unstable_by_key(|(_, e)| std::cmp::Reverse(e.0));
        for ((len, from, to), (n, src, dst)) in noted.into_iter().take(5) {
            let n = n as f64 / frames;
            if to.1 == 0 {
                tracing::info!("done on the card, per frame: {n:.2} fills of {len:#x} bytes from the host copy of {:#x}+{:#x} (last {src:#x})", from.0, from.1);
            } else {
                tracing::info!(
                    "done on the card, per frame: {n:.2} copies of {len:#x} bytes from the host copy of {:#x}+{:#x} (last {src:#x}) \
                     to that of {:#x}+{:#x} (last {dst:#x})",
                    from.0,
                    from.1,
                    to.0,
                    to.1
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Places of a host copy that follow one another as they do in the
    /// buffer between are read by one copy, gap and all; any other step
    /// between them keeps its own, and what each place is given is the
    /// same bytes of the same host copy as one copy each gave it.
    /// Places laid out by host copy and address ([`sorted_runs`]): each is
    /// given the bytes of its own place of its own host copy at its level,
    /// at a boundary; a run's bytes besides fall where no place lies; the
    /// places read mixed between host copies take a run per host copy; and
    /// in their order they lie where they always did.
    #[test]
    fn places_by_host_copy_are_read_in_runs() {
        // Two rings read in turn (a draw's constants from each), a place of
        // a third host copy read twice, a place inside another, one far off,
        // one off the boundaries, and the same places again at level 1.
        let mut places = Vec::new();
        for i in 0..6usize {
            places.push((1, 0x10000 + i * 512, 480, 0));
            places.push((2, 0x40000 + i * 256, 256, 0));
        }
        places.push((3, 0x9000, 64, 0));
        places.push((1, 0x10000 + 512, 128, 0));
        places.push((1, 0x10000 + 6 * 512 + 64 * 1024, 256, 0));
        places.push((2, 0x40000 + 6 * 256 + 16, 256, 0));
        places.push((1, 0x10000, 480, 1));
        places.push((2, 0x40000, 256, 1));
        let (ats, runs, end) = lay_out(&places, true);
        assert_eq!(runs.len(), 7);
        let mut seen = std::collections::HashSet::new();
        for (i, &(shadow, src, len, level)) in places.iter().enumerate() {
            let at = ats[i];
            assert_eq!(at % PLACE_ALIGN, 0);
            assert!(at + len as u64 <= end);
            // The one run that writes it, at its level, reads it from its place.
            let by: Vec<_> = runs.iter().filter(|r| r.1 <= at && at < r.1 + r.2).collect();
            assert_eq!(by.len(), 1);
            let &(first, base, bytes) = by[0];
            let (s0, a0, _, l0) = places[first];
            assert_eq!((s0, l0), (shadow, level));
            assert_eq!(a0 as u64 + (at - base), src as u64);
            assert!(at + len as u64 <= base + bytes);
            // Two places on the same bytes of the buffer between are the
            // same bytes of the same host copy.
            for (j, &(s1, src1, len1, l1)) in places.iter().enumerate() {
                let (b, e) = (ats[j], ats[j] + len1 as u64);
                if b < at + len as u64 && at < e {
                    assert_eq!((s1, l1), (shadow, level));
                    assert_eq!(src1 as i64 - src as i64, b as i64 - at as i64);
                }
            }
            seen.insert((shadow, level));
        }
        // No two runs share bytes.
        for (k, a) in runs.iter().enumerate() {
            for b in &runs[k + 1..] {
                assert!(a.1 + a.2 <= b.1 || b.1 + b.2 <= a.1);
            }
        }
        // In their order: one run each, where the order put them before.
        let (ats, runs, end) = lay_out(&places, false);
        assert_eq!(runs.len(), places.len());
        let mut was = 0u64;
        for (i, &(.., len, _)) in places.iter().enumerate() {
            assert_eq!(ats[i], was);
            assert_eq!(runs[i], (i, was, len as u64));
            was = (was + len as u64).next_multiple_of(PLACE_ALIGN);
        }
        assert_eq!(end.next_multiple_of(PLACE_ALIGN), was);
        assert_eq!(seen.len(), 5);
    }

    #[test]
    fn places_that_follow_one_another_are_read_in_runs() {
        // (source, host copy, offset, at, bytes): the ring of constants,
        // 480 bytes every 512, laid at 256 boundaries in the buffer between.
        let ring: Vec<_> = (0..4u64).map(|i| ('a', 1, 0x1000 + i * 512, i * 512, 480)).collect();
        assert_eq!(join_reads(ring.clone()), [('a', 0x1000, 0, 512 * 3 + 480)]);
        // A place that does not follow (another gap in the source, another
        // host copy, out of order, beyond the next boundary) stands alone.
        let mut reads = ring[..2].to_vec();
        reads.push(('a', 1, 0x1000 + 2 * 512 + 16, 1024, 480));
        reads.push(('b', 2, 0x1000 + 3 * 512 + 16, 1536, 480));
        reads.push(('b', 2, 0x1000 + 4 * 512 + 16, 2048 + 256, 480));
        reads.push(('b', 2, 0x0, 3072, 480));
        assert_eq!(
            join_reads(reads.clone()),
            [
                ('a', 0x1000, 0, 512 + 480),
                ('a', 0x1000 + 2 * 512 + 16, 1024, 480),
                ('b', 0x1000 + 3 * 512 + 16, 1536, 480),
                ('b', 0x1000 + 4 * 512 + 16, 2048 + 256, 480),
                ('b', 0x0, 3072, 480),
            ]
        );
        // Every place's bytes come from where they came from alone, and no
        // copy writes into another place.
        for reads in [ring, reads] {
            let joined = join_reads(reads.clone());
            for &(source, _, offset, at, len) in &reads {
                let by = joined.iter().filter(|j| j.2 <= at && at + len <= j.2 + j.3).collect::<Vec<_>>();
                assert_eq!(by.len(), 1);
                assert_eq!((by[0].0, by[0].1 + (at - by[0].2)), (source, offset));
            }
            for j in &joined {
                for &(_, _, _, at, len) in &reads {
                    let inside = j.2 <= at && at + len <= j.2 + j.3;
                    let apart = at + len <= j.2 || j.2 + j.3 <= at;
                    assert!(inside || apart);
                }
            }
        }
        // Ends that meet a boundary: no gap.
        let exact: Vec<_> = (0..3u64).map(|i| ('a', 1, i * 256, i * 256, 256)).collect();
        assert_eq!(join_reads(exact), [('a', 0, 0, 768)]);
    }

    fn copy(dst: usize, src: usize, len: usize, asked: bool) -> Does {
        Does::Copy { dst, src, len, asked }
    }

    fn fill(src: usize, len: usize, asked: bool) -> Does {
        Does::Fill { src, len, asked, to: true }
    }

    fn stuck(src: usize, len: usize) -> Does {
        Does::Fill { src, len, asked: false, to: false }
    }

    /// What is done on the card, as (place, level), in stage 2.
    fn planned(does: &[Does]) -> Vec<(usize, u32)> {
        plan(does, 2, true, &mut None).unwrap()
    }

    /// Only what is asked for is taken, all of it, and a step that stands
    /// in none of their ways stays where it is.
    #[test]
    fn steps_apart_from_one_another_are_taken() {
        let does = [
            copy(1000, 5000, 100, false),
            fill(2000, 64, true),
            Does::Refresh { shadow: 1, from: 0, to: 9000 },
            copy(3000, 6000, 100, true),
            fill(1000, 64, false),
            Does::Read { from: 3100, to: 4000, asked: false },
            stuck(3100, 5),
        ];
        assert_eq!(planned(&does), [(1, 0), (3, 0)]);
        // Two fills of the same bytes.
        assert_eq!(plan(&[fill(0, 64, true), fill(32, 64, true)], 1, true, &mut None), Ok(vec![(0, 0), (1, 0)]));
        assert_eq!(plan(&[fill(0, 64, false)], 1, true, &mut None), Ok(vec![]));
        // Ends that touch are apart.
        let does = [
            copy(1000, 5000, 100, true),
            copy(900, 9000, 100, false),
            copy(1100, 9000, 5, false),
            copy(4900, 9000, 100, false),
        ];
        assert_eq!(planned(&does), [(0, 0)]);
    }

    /// What depends on a taken step, or it on it, is taken too, on the level
    /// the order of the steps asks for.
    #[test]
    fn steps_that_depend_on_one_another_are_taken_in_order() {
        let taken = copy(1000, 5000, 100, true);
        // Reads what it writes, behind and ahead of it.
        assert_eq!(planned(&[taken, fill(1099, 64, false)]), [(0, 0), (1, 1)]);
        assert_eq!(planned(&[fill(1099, 64, false), taken]), [(0, 0), (1, 1)]);
        assert_eq!(planned(&[taken, copy(7000, 1050, 10, false)]), [(0, 0), (1, 1)]);
        // Writes what it reads, and what it writes.
        assert_eq!(planned(&[copy(5099, 9000, 1, false), taken]), [(0, 0), (1, 1)]);
        assert_eq!(planned(&[taken, copy(5099, 9000, 1, false)]), [(0, 0), (1, 1)]);
        assert_eq!(planned(&[taken, copy(999, 9000, 2, false)]), [(0, 0), (1, 1)]);
        // The same list twice.
        assert_eq!(planned(&[taken, taken]), [(0, 0), (1, 1)]);
        // A chain, with steps apart from it in between, and two fills of one
        // copy on one level.
        let does = [
            copy(8000, 7000, 50, false),
            taken,
            copy(2000, 1010, 20, false),
            fill(90_000, 10, false),
            fill(2000, 20, false),
            fill(2010, 20, true),
            copy(1000, 2005, 5, false),
            fill(1000, 100, false),
        ];
        assert_eq!(planned(&does), [(1, 0), (2, 1), (4, 2), (5, 2), (6, 2), (7, 3)]);
        // A fill is in nobody's way.
        assert_eq!(planned(&[taken, fill(5000, 100, false)]), [(0, 0)]);
        // A copy ahead that a taken one only shares its source with.
        assert_eq!(planned(&[copy(7000, 5000, 10, false), taken]), [(1, 0)]);
    }

    /// A copy at its place in a list is in the way of what reads or writes its
    /// bytes behind it, not of what did ahead of it.
    #[test]
    fn copies_at_their_place_stand_in_the_way_of_what_is_behind_them() {
        let placed = Does::Placed { dst: 2000, src: 3000, len: 64 };
        let behind = |does: &[Does]| {
            let mut found = None;
            let refused = plan(does, 2, true, &mut found);
            assert!(refused.is_ok() || found.is_some() && refused == Err(BEHIND_PLACED));
            assert_eq!(plan(does, 2, false, &mut None).is_err(), does.len() > 1 && !matches!(does, [_, Does::Fill { src: 5000, .. }]));
            found
        };
        assert_eq!(behind(&[fill(2032, 64, true), placed]), None);
        assert_eq!(behind(&[placed, fill(2032, 64, true)]), Some((0, 1)));
        assert_eq!(behind(&[placed, fill(5000, 64, true)]), None);
        assert_eq!(behind(&[fill(2000, 8, true), placed, fill(2000, 8, true)]), Some((1, 2)));
        // One that writes what it copies from, and one that writes over it.
        assert_eq!(behind(&[copy(3000, 5000, 8, true), placed]), None);
        assert_eq!(behind(&[placed, copy(3000, 5000, 8, true)]), Some((0, 1)));
        assert_eq!(behind(&[copy(2060, 5000, 8, true), placed]), None);
        assert_eq!(behind(&[placed, copy(2060, 5000, 8, true)]), Some((0, 1)));
    }

    /// Whatever the steps, done level by level (in a level every read
    /// before every write) they read and leave what they read and leave one
    /// after the other, the CPU's steps ahead of all of them.
    #[test]
    fn levels_do_what_the_order_does() {
        let mut seed = 12345u32;
        let mut random = move |n: u32| {
            seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12345);
            (seed >> 16) % n
        };
        let (mut done, mut refused) = (0, 0);
        for _ in 0..4000 {
            let does: Vec<Does> = (0..2 + random(9))
                .map(|_| {
                    let (at, other, len) = (random(60) as usize, random(60) as usize, 1 + random(8) as usize);
                    let asked = random(4) == 0;
                    match random(8) {
                        0..4 => copy(at, other, len, asked),
                        4..6 => fill(at, len, asked),
                        6 => stuck(at, len),
                        _ => Does::Read { from: at, to: at + len, asked: false },
                    }
                })
                .collect();
            let Ok(chosen) = plan(&does, 2, true, &mut None) else {
                refused += 1;
                continue;
            };
            done += 1;
            let start: Vec<u32> = (0..80).collect();
            // One after the other: what each reads, and what is left.
            let mut memory = start.clone();
            let mut wanted = Vec::new();
            for d in &does {
                let (from, to) = match *d {
                    Does::Read { from, to, .. } => (from, to),
                    _ => reads_writes(d).unwrap().0,
                };
                wanted.push(memory[from..to].to_vec());
                if let Does::Copy { dst, src, len, .. } = *d {
                    memory.copy_within(src..src + len, dst);
                }
            }
            // The CPU's, then the card's.
            let mut cpu = start.clone();
            let taken = |i: usize| chosen.iter().any(|c| c.0 == i);
            for (i, d) in does.iter().enumerate().filter(|&(i, _)| !taken(i)) {
                let (from, to) = match *d {
                    Does::Read { from, to, .. } => (from, to),
                    _ => reads_writes(d).unwrap().0,
                };
                assert_eq!(cpu[from..to], wanted[i][..], "step {i} of {does:?}");
                if let Does::Copy { dst, src, len, .. } = *d {
                    cpu.copy_within(src..src + len, dst);
                }
            }
            let mut card = cpu.clone();
            for level in 0..=chosen.iter().map(|c| c.1).max().unwrap_or(0) {
                let of: Vec<usize> = chosen.iter().filter(|c| c.1 == level).map(|c| c.0).collect();
                let before = card.clone();
                for i in of {
                    let ((from, to), (dst, _)) = reads_writes(&does[i]).unwrap();
                    assert_eq!(before[from..to], wanted[i][..], "step {i} of {does:?}: {chosen:?}");
                    if matches!(does[i], Does::Copy { .. }) {
                        card[dst..dst + to - from].copy_from_slice(&before[from..to]);
                    }
                }
            }
            assert_eq!(card, memory, "{does:?}: {chosen:?}");
        }
        assert!(done > 500 && refused > 100, "{done} {refused}");
    }

    /// What cannot be done on the card refuses the lot, where it stands in
    /// the way.
    #[test]
    fn a_fill_behind_a_copy_at_its_place_reads_where_the_copy_does() {
        let fill = |src| Does::Fill { src, len: 0x100, asked: true, to: true };
        let mut does = vec![
            fill(0x1000),
            Does::Placed { dst: 0x1000, src: 0x9000, len: 0x200 },
            fill(0x1080),
            fill(0x1180),
        ];
        // Ahead of the copy: as it was. Behind it and inside: from the source. Partly outside: as it was.
        assert_eq!(read_behind_placed(&mut does, &[(0x1000, 0x1200)]), 1);
        assert!(matches!(does[0], Does::Fill { src: 0x1000, .. }));
        assert!(matches!(does[2], Does::Fill { src: 0x9080, .. }));
        assert!(matches!(does[3], Does::Fill { src: 0x1180, .. }));
        // The one partly outside is still in the copy's way; without it nothing is.
        assert_eq!(plan(&does, 2, true, &mut None), Err(BEHIND_PLACED));
        does.pop();
        assert!(plan(&does, 2, true, &mut None).is_ok());
        // A buffer copy between them that writes what the fill reads: left alone.
        let mut does = vec![
            Does::Placed { dst: 0x1000, src: 0x9000, len: 0x200 },
            Does::Copy { dst: 0x1040, src: 0x5000, len: 0x80, asked: false },
            fill(0x1080),
        ];
        assert_eq!(read_behind_placed(&mut does, &[(0x1000, 0x1200)]), 0);
        // Nothing in the air writes any of it: the CPU's fill is right.
        let mut does = vec![Does::Placed { dst: 0x1000, src: 0x9000, len: 0x200 }, Does::Fill { src: 0x1080, len: 0x100, asked: false, to: true }];
        assert_eq!(read_behind_placed(&mut does, &[(0x4000, 0x5000)]), 0);
        // Two copies in a row: the bytes are followed back through both.
        let mut does = vec![
            Does::Placed { dst: 0x9000, src: 0x7000, len: 0x200 },
            Does::Placed { dst: 0x1000, src: 0x9000, len: 0x200 },
            fill(0x1080),
        ];
        assert_eq!(read_behind_placed(&mut does, &[(0x1000, 0x1200)]), 1);
        assert!(matches!(does[2], Does::Fill { src: 0x7080, .. }));
    }

    /// [`card_fills`]: the fills around a copy at its place that the card does read the bytes
    /// the CPU's fills read after the wait for the flights, step by step; what would read
    /// others is refused (and waits, as before).
    #[test]
    fn fills_around_a_copy_at_its_place_read_what_waiting_reads() {
        let all = &mut |_: usize, _: usize| true;
        // Run r90: a copy at its place puts 0xbd0 bytes where flights declare writes, and a
        // fill behind it reads 0xf0 of them. Without the switch refused; with it the fill reads
        // the copy's source. Where title memory is not all mapped, refused as before.
        let placed = Does::Placed { dst: 0x1000, src: 0x9000, len: 0xbd0 };
        let written = [(0x1000, 0x1bd0)];
        let mut does = vec![placed, fill(0x1000, 0xf0, true)];
        assert_eq!(plan_with(&does, 2, false, false, &mut None), Err(BEHIND_PLACED));
        assert_eq!(read_behind_placed_where(&mut does.clone(), &written, &mut |_, _| false), 0);
        assert_eq!(read_behind_placed_where(&mut does, &written, all), 1);
        assert!(matches!(does[1], Does::Fill { src: 0x9000, len: 0xf0, asked: true, .. }));
        assert_eq!(plan_with(&does, 2, false, true, &mut None), Ok(vec![(1, 0)]));
        assert!(!read_behind_on_cpu(&does, &written));
        // A fill ahead of it reads what was there before, in front of the lists as well.
        let does = [fill(0x1000, 0xf0, true), placed];
        assert_eq!(plan_with(&does, 2, false, false, &mut None), Err(BEHIND_PLACED));
        assert_eq!(plan_with(&does, 2, false, true, &mut None), Ok(vec![(0, 0)]));
        // A copy done on the card that writes its source or destination: still in its way.
        for taken in [copy(0x9000, 0x5000, 8, true), copy(0x1100, 0x5000, 8, true)] {
            assert_eq!(plan_with(&[taken, fill(0x1000, 8, true), placed], 2, false, true, &mut None), Err(BEHIND_PLACED));
        }
        // A texture behind it reads its destination on the CPU, which title memory does not
        // have while flights write its source.
        let does = [placed, Does::Read { from: 0x1010, to: 0x1020, asked: false }, fill(0x4000, 8, true)];
        assert!(plan_with(&does, 2, false, true, &mut None).is_ok());
        assert!(read_behind_on_cpu(&does, &[(0x4000, 0x4008), (0x9000, 0x9bd0)]));
        assert!(!read_behind_on_cpu(&does, &[(0x4000, 0x4008)]));

        // Any steps: what the card and the CPU read is what the CPU read after the wait.
        let mut seed = 4242u32;
        let mut random = move |n: u32| {
            seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12345);
            (seed >> 16) % n
        };
        let (mut done, mut refused, mut redirected, mut gained) = (0, 0, 0, 0);
        for _ in 0..20_000 {
            let mut written = vec![(random(40) as usize, 0)];
            written[0].1 = written[0].0 + 1 + random(16) as usize;
            if random(2) == 0 {
                let at = written[0].1 + 1 + random(10) as usize;
                written.push((at, at + 1 + random(10) as usize));
            }
            let does: Vec<Does> = (0..2 + random(6))
                .map(|_| {
                    let (at, len) = (random(56) as usize, 1 + random(8) as usize);
                    match random(5) {
                        0 | 1 => {
                            let mut src = random(56) as usize;
                            while meet((src, src + len), (at, at + len)) {
                                src = (src + 9) % 56;
                            }
                            Does::Placed { dst: at, src, len }
                        }
                        2 | 3 => fill(at, len, touches(&written, at, at + len)),
                        _ => Does::Read { from: at, to: at + len, asked: touches(&written, at, at + len) },
                    }
                })
                .collect();
            let range = |d: &Does| match *d {
                Does::Fill { src, len, .. } => Some((src, src + len)),
                Does::Read { from, to, .. } => Some((from, to)),
                _ => None,
            };
            // Title memory before the flights land, and after (they wrote 100 and up).
            let before: Vec<u32> = (0..64).collect();
            let landed: Vec<u32> = (0..64u32).map(|i| if touches(&written, i as usize, i as usize + 1) { 100 + i } else { i }).collect();
            // Waiting: the CPU's steps one after the other, behind the landing.
            let mut memory = landed.clone();
            let mut wanted = Vec::new();
            for d in &does {
                wanted.push(range(d).map(|(a, b)| memory[a..b].to_vec()));
                if let Does::Placed { dst, src, len } = *d {
                    memory.copy_within(src..src + len, dst);
                }
            }
            // Not waiting: the card in front of the lists, what it reads landed; the CPU's
            // steps in title memory as it is, a copy at its place put there only where no
            // flight writes its bytes (`place::Placed::prepare`).
            let mut on_card = does.clone();
            let moved = read_behind_placed_where(&mut on_card, &written, all);
            let chosen = match plan_with(&on_card, 2, false, true, &mut None) {
                Ok(chosen) if !chosen.is_empty() && !read_behind_on_cpu(&on_card, &written) => chosen,
                Ok(chosen) if chosen.is_empty() => continue,
                _ => {
                    refused += 1;
                    continue;
                }
            };
            done += 1;
            redirected += moved;
            // (Refused without the switch.)
            gained += usize::from(plan_with(&does, 2, false, false, &mut None) == Err(BEHIND_PLACED));
            let mut cpu = before.clone();
            for (i, d) in on_card.iter().enumerate() {
                let taken = chosen.iter().any(|c| c.0 == i);
                if let Some((a, b)) = range(d) {
                    let read = if taken { &landed[a..b] } else { &cpu[a..b] };
                    assert_eq!(Some(read.to_vec()), wanted[i], "step {i} of {does:?} as {on_card:?}, flights write {written:?}");
                }
                if let Does::Placed { dst, src, len } = *d
                    && !touches(&written, src, src + len)
                    && !touches(&written, dst, dst + len)
                {
                    cpu.copy_within(src..src + len, dst);
                }
            }
        }
        assert!(done > 2000 && refused > 1000 && redirected > 100 && gained > 300, "{done} {refused} {redirected} {gained}");
    }

    #[test]
    fn steps_in_the_way_are_refused() {
        let taken = copy(1000, 5000, 100, true);
        assert!(plan(&[taken, Does::Run], 2, true, &mut None).is_err());
        // A texture that itself waits for flights; a fill that cannot be
        // taken but waits.
        assert!(plan(&[taken, Does::Read { from: 1, to: 2, asked: true }], 2, true, &mut None).is_err());
        assert!(plan(&[Does::Fill { src: 1, len: 1, asked: true, to: false }], 2, true, &mut None).is_err());
        // Reads on the CPU of what it writes: behind it, not ahead of it.
        for read in [Does::Read { from: 900, to: 1001, asked: false }, stuck(1050, 3)] {
            assert!(plan(&[taken, read], 2, true, &mut None).is_err(), "{read:?} behind");
            assert_eq!(planned(&[read, taken]), [(1, 0)], "{read:?} ahead");
        }
        // Also of what is taken along.
        assert!(plan(&[taken, copy(3000, 1000, 10, false), stuck(3005, 1)], 2, true, &mut None).is_err());
        // Into itself, and copies without stage 2.
        assert!(plan(&[copy(1000, 1050, 100, true)], 2, true, &mut None).is_err());
        assert!(plan(&[taken], 1, true, &mut None).is_err());
        assert!(plan(&[fill(1000, 4, true), copy(1000, 5000, 100, false)], 1, true, &mut None).is_err());
    }

    #[test]
    fn a_refresh_of_another_host_copy_is_seen() {
        let does = [Does::Refresh { shadow: 7, from: 100, to: 200 }];
        assert!(!elsewhere(&does, 7, 100, 200));
        assert!(elsewhere(&does, 8, 199, 300));
        assert!(!elsewhere(&does, 8, 200, 300));
    }

    /// A value written behind a flight, or a read-back, is in no host copy
    /// before it lands.
    #[test]
    fn what_no_host_copy_holds_is_refused() {
        let writers = [Writer { shadow: None, from: 100, to: 108, copy: false }];
        assert!(holder(&writers, 104, 120).is_err());
    }

    /// [`fast`]: the index of what flights write finds what the walk over
    /// all of it found, the same host copy or the same refusal, also with
    /// copies done here added behind it.
    #[test]
    fn the_index_finds_what_the_walk_finds() {
        let mut seed = 777u32;
        let mut random = move |n: u32| {
            seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12345);
            (seed >> 16) % n
        };
        let (mut found, mut refused, mut none) = (0, 0, 0);
        for round in 0..3000 {
            let writer = |random: &mut dyn FnMut(u32) -> u32| {
                let s = [0, 0x10, 0x10, 0x20, 0x20, 0x20, 0x30][random(7) as usize];
                let from = random(300) as usize;
                let to = match random(20) {
                    0 => from + 100_000,
                    1 => from.saturating_sub(3),
                    _ => from + random(40) as usize,
                };
                (s, from, to)
            };
            // Few writers, or many of one host copy.
            let n = if round % 4 == 0 { 300 } else { random(30) };
            let mut all: Vec<(usize, usize, usize)> = (0..n).map(|_| writer(&mut random)).collect();
            if round % 4 == 0 {
                all.iter_mut().for_each(|w| w.0 = if w.0 == 0 && random(50) != 0 { 0x10 } else { w.0 });
            }
            let mut index = Writers { all: all.clone(), ..Writers::default() };
            index.index();
            for _ in 0..random(3) {
                let w = writer(&mut random);
                all.push(w);
                index.all.push(w);
            }
            for _ in 0..30 {
                let from = random(340) as usize;
                let to = from + random(50) as usize;
                let walked = holding(all.iter().copied(), from, to);
                assert_eq!(index.holding(from, to), walked, "[{from}, {to}) in {all:?}");
                match walked {
                    Ok(0) => none += 1,
                    Ok(_) => found += 1,
                    Err(_) => refused += 1,
                }
            }
        }
        assert!(found > 1000 && refused > 1000 && none > 1000, "{found} {refused} {none}");
    }

    /// Reads are all before writes, between two buffers and within one, and
    /// the buffers are usable in the lists that follow in the same call.
    #[test]
    fn the_card_copies_what_is_recorded() {
        let Some(host) = host() else {
            return; // no host GPU
        };
        let (a, a_data) = host.shared_buffer(1 << 16).unwrap();
        let (b, b_data) = host.shared_buffer(1 << 16).unwrap();
        // SAFETY: mappings of 64 KiB.
        let (a_bytes, b_bytes) =
            unsafe { (std::slice::from_raw_parts_mut(a_data, 1 << 16), std::slice::from_raw_parts_mut(b_data, 1 << 16)) };
        for (i, (x, y)) in a_bytes.iter_mut().zip(b_bytes.iter_mut()).enumerate() {
            (*x, *y) = (i as u8 ^ (i >> 8) as u8, 0xee);
        }
        let was = a_bytes.to_vec();
        let own = host.own_list().unwrap();
        // Within `a` (there and back), from `a` to `b`, and a read alone.
        record(
            true,
            &own.list.list,
            &own.between,
            &[(&a, 33, 0, 100), (&a, 2000, 256, 100), (&a, 5001, 512, 7), (&a, 9, 1024, 4)],
            &[(&a, 2000, 0, 100), (&a, 33, 256, 100), (&b, 77, 512, 7)],
        );
        // A level behind it, of what it wrote.
        record(false, &own.list.list, &own.between, &[(&b, 77, 2048, 7)], &[(&b, 1077, 2048, 7)]);
        own.list.close();
        // A list behind it that uses the buffers in other ways.
        let next = host.new_list().unwrap();
        let (out, out_data) = host.shared_buffer(1 << 16).unwrap();
        // SAFETY: an open list; live buffers.
        unsafe {
            next.list.CopyBufferRegion(&out, 0, &own.between, 1024, 4);
            next.list.CopyBufferRegion(&b, 4096, &a, 33, 100);
        }
        next.close();
        host.execute(&[&own.list, &next]);
        // With the debug layer (`VICTORY_D3D12_DEBUG=1`, diagnostics).
        assert_eq!(host.log_messages(), None);
        assert_eq!(a_bytes[2000..2100], was[33..133]);
        assert_eq!(a_bytes[33..133], was[2000..2100]);
        assert_eq!(b_bytes[77..84], was[5001..5008]);
        assert_eq!(b_bytes[1077..1084], was[5001..5008]);
        assert_eq!(b_bytes[4096..4196], was[2000..2100]);
        // SAFETY: a mapping of 64 KiB.
        assert_eq!(unsafe { std::slice::from_raw_parts(out_data, 4) }, &was[9..13]);
        assert!(b_bytes[..77].iter().all(|&x| x == 0xee));
        a_bytes[2000..2100].copy_from_slice(&was[2000..2100]);
        a_bytes[33..133].copy_from_slice(&was[33..133]);
        assert_eq!(a_bytes, &was[..]);
        // As the landing does: its next use records anew.
        POOL.lock().unwrap().push(own);
        let own = host.own_list().unwrap();
        own.list.close();
    }

    /// The whole way, in a process of its own with the relay on:
    /// `VICTORY_RELAY=2 VICTORY_CARD_STEPS=2 cargo test -p victory-gpu --lib
    /// card_steps -- --ignored --test-threads=1`. A flight's list writes
    /// part of what the next submission's copy reads, the title wrote the
    /// rest; the copy is done on the card behind the flight, and a second
    /// one reads what the first wrote. Title memory ends as on the CPU.
    #[test]
    #[ignore = "wants VICTORY_RELAY=2 VICTORY_CARD_STEPS=2 and a process of its own"]
    fn a_copy_behind_a_flight_reads_what_the_flight_wrote() {
        use windows_sys::Win32::System::Memory::{MEM_COMMIT, MEM_RESERVE, PAGE_READWRITE, VirtualAlloc};
        const LEN: usize = 4 << 20;
        // Without it (`TEST_UNWATCHED=1`) the title's bytes cannot be put in
        // under a flight's writes: the flights that declared them are waited
        // for.
        let write_watch = if std::env::var_os("TEST_UNWATCHED").is_some() { 0 } else { 0x20_0000 };
        let Some(host) = host() else {
            return;
        };
        assert!(relay_exact() && stage() == 2, "set VICTORY_RELAY=2 VICTORY_CARD_STEPS=2");
        // Two title buffers: in the first a copy within it, from it into the
        // second, and back.
        let (title, start) = {
            // SAFETY: memory of our own, never freed.
            let base = unsafe { VirtualAlloc(std::ptr::null(), LEN, MEM_RESERVE | MEM_COMMIT | write_watch, PAGE_READWRITE) }.cast::<u8>();
            assert!(!base.is_null());
            // SAFETY: `LEN` bytes just committed.
            (unsafe { std::slice::from_raw_parts_mut(base, LEN) }, base as usize)
        };
        const HALF: usize = LEN / 2;
        crate::shadow::register(0x1111, start as u64, HALF as u64);
        crate::shadow::register(0x2222, (start + HALF) as u64, HALF as u64);
        let first = crate::shadow::find(start as u64, 64).unwrap();
        let (slow_from, slow_to) = (host.uav_buffer(64 << 20).unwrap(), host.uav_buffer(64 << 20).unwrap());
        let before = DONE[1].load(Ordering::Relaxed);
        let mut lists = Vec::new();
        for round in 0..8usize {
            // The title writes; then a "shader" of a flight writes 300 bytes
            // of what the copy reads, behind some work.
            let seed = |i: usize| (i * 7 + round * 13 + i / 251) as u8;
            for (i, b) in title[4000..6000].iter_mut().enumerate() {
                *b = seed(i);
            }
            let (upload, data) = host.upload_buffer(4096).unwrap();
            for i in 0..300 {
                // SAFETY: inside the mapping.
                unsafe { data.add(i).write(0x80 | (round * 31 + i) as u8) };
            }
            let a = host.new_list().unwrap();
            first.refresh(&a, start as u64, HALF as u64);
            first.gpu_writes(&a, start as u64, HALF as u64);
            // SAFETY: an open list; live buffers.
            unsafe {
                for _ in 0..24 {
                    a.list.CopyBufferRegion(&slow_to, 0, &slow_from, 0, 64 << 20);
                }
                a.list.CopyBufferRegion(&first.buffer, 4500, &upload, 0, 300);
                a.list
                    .ResourceBarrier(&[transition(&first.buffer, D3D12_RESOURCE_STATE_COPY_DEST, D3D12_RESOURCE_STATE_COMMON)]);
            }
            a.close();
            host.execute_own(&a);
            // With the flight in the air, beside what it writes.
            for (i, b) in title[5300..5400].iter_mut().enumerate() {
                *b = seed(1300 + i) ^ 0x5c;
            }
            // Thousands of runs of bytes of its own.
            for b in title[1_700_000..1_708_000].iter_mut().step_by(2) {
                *b = 1 + round as u8;
            }
            // Into the second buffer, and within the first.
            let b = host.new_list().unwrap();
            b.defer_copy((start + HALF + 100) as *mut u8, (start + 4001) as *const u8, 1500);
            b.defer_copy((start + 900_001) as *mut u8, (start + 4400) as *const u8, 777);
            b.defer_copy((start + HALF + 10_000) as *mut u8, (start + 1_700_000) as *const u8, 8000);
            // What depends on them: a copy of the first copy, upload memory
            // filled from that, and the first one's bytes copied over.
            b.defer_copy((start + 1_500_000) as *mut u8, (start + HALF + 600) as *const u8, 700);
            let (staging, staged) = b.upload(host, 4096).unwrap();
            b.defer_fill_buffer(&staging, staged, (start + 1_500_100) as *const u8, 500);
            b.defer_copy((start + HALF + 300) as *mut u8, (start + 8000) as *const u8, 64);
            b.close();
            host.execute_own(&b);
            // What the first wrote, read by the next submission.
            let c = host.new_list().unwrap();
            c.defer_copy((start + 1_200_000) as *mut u8, (start + HALF + 100) as *const u8, 1500);
            c.close();
            let number = host.execute_own(&c);
            settle_now(WHY_SYNC);
            assert!(FINISHED.load(Ordering::Acquire) >= number);
            let wanted: Vec<u8> = (0..2000)
                .map(|i| match i {
                    500..800 => 0x80 | (round * 31 + i - 500) as u8,
                    1300..1400 => seed(i) ^ 0x5c,
                    _ => seed(i),
                })
                .collect();
            assert!(title[4000..6000] == wanted[..], "what the flight wrote, round {round}");
            // (The copy over the first one's bytes came last.)
            let mut over = wanted[1..1501].to_vec();
            over[200..264].fill(0);
            assert!(title[HALF + 100..HALF + 1600] == over[..], "the copy to the other buffer, round {round}");
            assert!(title[1_500_000..1_500_700] == wanted[501..1201], "the copy behind a copy, round {round}");
            // SAFETY: the mapping, which the flight wrote and has landed.
            let staged = unsafe { std::slice::from_raw_parts(staged, 500) };
            assert!(staged == &wanted[601..1101], "the fill behind it, round {round}");
            assert!(title[900_001..900_778] == wanted[400..1177], "the copy within the buffer, round {round}");
            assert!(title[1_200_000..1_201_500] == over[..], "the copy of the copy, round {round}");
            assert!(title[HALF..HALF + 100].iter().all(|&x| x == 0));
            assert!(title[HALF + 1600..HALF + 10_000].iter().all(|&x| x == 0));
            assert!(title[HALF + 18_000..].iter().all(|&x| x == 0));
            let runs = &title[HALF + 10_000..HALF + 18_000];
            assert!(runs.chunks(2).all(|b| b == [1 + round as u8, 0]), "the runs, round {round}");
            a.keep(&upload);
            lists.extend([a, b, c]);
        }
        assert_eq!(host.log_messages(), None);
        let done = DONE[1].load(Ordering::Relaxed) - before;
        eprintln!("{done} of 48 copies were done on the card, {} submissions waited", DONE[3].load(Ordering::Relaxed));
        eprintln!("settles for title memory under shader writes: {}", LATE_STATS[WHY_STALE].load(Ordering::Relaxed));
        assert!(done >= 20, "only {done} copies were done on the card");
    }
}

