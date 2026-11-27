//! Work of the submitting thread moved to other threads
//! (`VICTORY_SPLIT_SUBMIT=1`; off by default), see [`on`].

use std::collections::HashSet;
use std::hash::BuildHasherDefault;
use std::sync::{Condvar, Mutex, OnceLock};

use super::{AddressHash, ByCopy};

/// Whether the submitting thread (the lane of `VICTORY_ASYNC_SUBMIT` /
/// `VICTORY_THIN`, `crate::queue::behind`, in [`super::Host::submit`]) hands
/// work that does not depend on the flights in the air to other threads
/// (`VICTORY_SPLIT_SUBMIT=1`; off by default).
///
/// In the heaviest scenes (5,000-9,000 draws, 35-40 submissions a frame) that
/// one thread works 30-40 ms a frame and the title waits for it at presents.
///
/// # Why not two submissions prepared at once
///
/// The obvious split, the next submission (N+1) prepared on a second thread
/// while N is still being prepared or sent, was looked at and not done. Its
/// preparing reads and writes state that exists once, for one submission at a
/// time, not per submission:
///
/// - the write watch's one asking (`crate::watch::ask_once`): one journal
///   asking "stands" for the submission being prepared, and the next one's
///   asking ends it;
/// - the host copies' snapshots and the ranges declared written
///   (`Shadow::writing`, `crate::shadow::take_snapshots`), per host copy;
/// - what flights in the air may write: `line().written`, which
///   [`super::late_blocked`], [`super::stale_under_flights`] and
///   [`super::card_steps`] read, holds N's ranges only once N's flight is in
///   the line, after it was sent.
///
/// soldier16's two-stage submit (s16 67329b6) released the submission lock
/// before the flight was in the line: N+1 could be prepared against a line
/// without N's writes, and refresh a range N's shaders write (or skip the
/// wait for it). Asking the write watch for N+1 early does not help either:
/// an asking costs the memory it walks, not what it finds, and an early
/// answer cannot stand for N+1's own, since our landings write title memory
/// in between.
///
/// # What is done instead
///
/// Only work that is a pure function of a closed list leaves the submitting
/// thread, and the submitting thread still does everything else in the same
/// order as without the switch:
///
/// 1. A list's summary ([`Summary`]) is made when the list is closed, on the
///    thread that closes it (the title's recording threads, or ours with
///    `VICTORY_THIN`): per host copy the span of its refreshes, and the
///    ranges its shaders are declared to write, each once. The submission
///    then merges summaries ([`add_spans`], [`Declaring`]) instead of walking
///    every step of every list (20,000 refreshes and thousands of declared
///    writes a frame) for its write-watch asking, its refreshes in front
///    (`Batch::copy_spans`) and its copy-back (`declared_writes`).
/// 2. The landed lists' reset and return (`land`) after each sending goes to
///    a thread of its own ([`land_beside`]), as the thread that settles
///    flights already does it.
///
/// # Invariants (why it is race-free)
///
/// - Nothing about flights moves: preparing still happens under the
///   submission lock, N's flight (its written ranges, its fence value) is in
///   the line before the lock is let go, and only then can N+1 take it and
///   look at `line().written` (`late_blocked`, `stale_under_flights`,
///   `card_steps`). The order of `ExecuteCommandLists` and fence signals is
///   untouched: one thread, one lock, as before.
/// - A summary is the result the walk would give, exactly: the same host
///   copies in the same order, the same spans, the same declared ranges in
///   the same order, each once (`tests` compare the two on random steps).
///   It reads nothing but the list's own steps, which nobody changes once the
///   list is closed; it is kept with the steps (`Deferred`), so a reset drops
///   it, and it carries the number of steps it was made of: a list with more
///   steps since (a step added after the close) is walked as before.
/// - What `land` does has no bearing on the next submission's preparing: it
///   resets lists that have run and landed, which no submission names, and
///   another thread already does the same at other times.
pub(crate) fn on() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| crate::switch::on("VICTORY_SPLIT_SUBMIT"))
}

/// A step of a list, as far as a summary cares.
pub(super) enum Seen<'a, T> {
    /// A refresh of a host copy: address, bytes.
    Refresh(&'a T, usize, usize),
    /// A declared shader write into a host copy: address, bytes.
    Write(&'a T, usize, usize),
    Other,
}

/// What a closed list's steps say about the host copies it refreshes and
/// writes (see [`on`]).
pub(super) struct Summary<T> {
    /// The number of the list's steps it was made of.
    pub(super) steps: usize,
    /// Per host copy, in the order of its first refresh: the span of all its
    /// refreshes (from, to).
    pub(super) spans: Vec<(T, usize, usize)>,
    /// Per host copy, in the order of its first declared write: the ranges
    /// (from, to) declared, each once, in their order.
    pub(super) declared: Vec<(T, Vec<(usize, usize)>)>,
}

type Ranges = HashSet<(usize, usize, usize), BuildHasherDefault<AddressHash>>;

impl<T: Clone> Summary<T> {
    /// Of a list of `steps` steps, `seen`; `key` tells host copies apart.
    pub(super) fn of<'a>(steps: usize, seen: impl IntoIterator<Item = Seen<'a, T>>, key: impl Fn(&T) -> usize) -> Self
    where
        T: 'a,
    {
        let mut spans = Vec::new();
        let mut spans_at: ByCopy<usize> = ByCopy::default();
        let mut declaring = Declaring::default();
        for step in seen {
            match step {
                Seen::Refresh(copy, at, len) => span(&mut spans, &mut spans_at, copy, at, at + len, &key),
                Seen::Write(copy, at, len) => declaring.one(copy, at, at + len, &key),
                Seen::Other => {}
            }
        }
        Summary {
            steps,
            spans,
            declared: declaring.done(),
        }
    }
}

/// Widens `copy`'s span in `spans` (where each copy is: `at`) by
/// `[from, to)`, or adds it.
fn span<T: Clone>(
    spans: &mut Vec<(T, usize, usize)>,
    at: &mut ByCopy<usize>,
    copy: &T,
    from: usize,
    to: usize,
    key: &impl Fn(&T) -> usize,
) {
    let i = *at.entry(key(copy)).or_insert_with(|| {
        spans.push((copy.clone(), from, to));
        spans.len() - 1
    });
    let s = &mut spans[i];
    (s.1, s.2) = (s.1.min(from), s.2.max(to));
}

/// Adds a list's spans (of its [`Summary`]) to a submission's: what walking
/// its refreshes after the earlier lists' would have made.
pub(super) fn add_spans<T: Clone>(
    spans: &mut Vec<(T, usize, usize)>,
    at: &mut ByCopy<usize>,
    of: &[(T, usize, usize)],
    key: impl Fn(&T) -> usize,
) {
    for (copy, from, to) in of {
        span(spans, at, copy, *from, *to, &key);
    }
}

/// The declared shader writes of a submission, gathered list by list: per
/// host copy in the order of its first new range, each range once.
pub(super) struct Declaring<T> {
    back: Vec<(T, Vec<(usize, usize)>)>,
    places: ByCopy<usize>,
    seen: Ranges,
}

impl<T> Default for Declaring<T> {
    fn default() -> Self {
        Declaring {
            back: Vec::new(),
            places: ByCopy::default(),
            seen: Ranges::default(),
        }
    }
}

impl<T: Clone> Declaring<T> {
    /// One declared write, `[from, to)` of `copy`.
    pub(super) fn one(&mut self, copy: &T, from: usize, to: usize, key: &impl Fn(&T) -> usize) {
        let k = key(copy);
        if !self.seen.insert((k, from, to)) {
            return;
        }
        let i = *self.places.entry(k).or_insert_with(|| {
            self.back.push((copy.clone(), Vec::new()));
            self.back.len() - 1
        });
        self.back[i].1.push((from, to));
    }

    /// A list's, from its [`Summary`]. (A copy none of whose ranges are new
    /// gets no place: in the walk its place comes with its first new range,
    /// and a copy without one before this list has all of its ranges new.)
    pub(super) fn list(&mut self, of: &[(T, Vec<(usize, usize)>)], key: impl Fn(&T) -> usize) {
        for (copy, ranges) in of {
            for &(from, to) in ranges {
                self.one(copy, from, to, &key);
            }
        }
    }

    pub(super) fn done(self) -> Vec<(T, Vec<(usize, usize)>)> {
        self.back
    }
}

/// Has [`super::land`] done on a thread of its own, soon (see [`on`]).
pub(super) fn land_beside() {
    static ASKED: (Mutex<bool>, Condvar) = (Mutex::new(false), Condvar::new());
    static STARTED: OnceLock<bool> = OnceLock::new();
    let started = *STARTED.get_or_init(|| {
        std::thread::Builder::new()
            .name("victory-land".into())
            .spawn(|| {
                loop {
                    {
                        let mut asked = ASKED.0.lock().unwrap_or_else(|e| e.into_inner());
                        while !*asked {
                            asked = ASKED.1.wait(asked).unwrap_or_else(|e| e.into_inner());
                        }
                        *asked = false;
                    }
                    let _landing = crate::timeline::span("landed lists reset and given back (VICTORY_SPLIT_SUBMIT)");
                    super::land();
                }
            })
            .is_ok()
    });
    if !started {
        // No thread for it: here, as without the switch.
        super::land();
        return;
    }
    *ASKED.0.lock().unwrap_or_else(|e| e.into_inner()) = true;
    ASKED.1.notify_one();
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fake host copy: its number is its key.
    type Copy = usize;

    #[derive(Clone, Copy, Debug)]
    enum Fake {
        Refresh(Copy, usize, usize),
        Write(Copy, usize, usize),
        Other,
    }

    fn seen(steps: &[Fake]) -> impl Iterator<Item = Seen<'_, Copy>> {
        steps.iter().map(|s| match s {
            Fake::Refresh(c, at, len) => Seen::Refresh(c, *at, *len),
            Fake::Write(c, at, len) => Seen::Write(c, *at, *len),
            Fake::Other => Seen::Other,
        })
    }

    /// `Batch::copy_spans` as it walks every step of every list.
    fn walked_spans(lists: &[Vec<Fake>]) -> Vec<(Copy, usize, usize)> {
        let mut spans: Vec<(Copy, usize, usize)> = Vec::new();
        let mut at: std::collections::HashMap<usize, usize> = Default::default();
        let mut last = (0, 0);
        for list in lists {
            for step in list {
                let Fake::Refresh(copy, address, len) = *step else {
                    continue;
                };
                let (from, to) = (address, address + len);
                let key = copy;
                let i = if last.0 == key && key != 0 {
                    last.1
                } else {
                    *at.entry(key).or_insert_with(|| {
                        spans.push((copy, from, to));
                        spans.len() - 1
                    })
                };
                last = (key, i);
                let s = &mut spans[i];
                (s.1, s.2) = (s.1.min(from), s.2.max(to));
            }
        }
        spans
    }

    /// `declared_writes` as it walks every step of every list.
    fn walked_declared(lists: &[Vec<Fake>]) -> Vec<(Copy, Vec<(usize, usize)>)> {
        let mut back: Vec<(Copy, Vec<(usize, usize)>)> = Vec::new();
        let mut places: std::collections::HashMap<usize, usize> = Default::default();
        let mut seen = std::collections::HashSet::new();
        for list in lists {
            for step in list {
                if let Fake::Write(copy, address, len) = *step {
                    if !seen.insert((copy, address, len)) {
                        continue;
                    }
                    let i = *places.entry(copy).or_insert_with(|| {
                        back.push((copy, Vec::new()));
                        back.len() - 1
                    });
                    back[i].1.push((address, address + len));
                }
            }
        }
        back
    }

    fn merged(lists: &[Vec<Fake>]) -> (Vec<(Copy, usize, usize)>, Vec<(Copy, Vec<(usize, usize)>)>) {
        let sums: Vec<Summary<Copy>> = lists.iter().map(|l| Summary::of(l.len(), seen(l), |c| *c)).collect();
        let (mut spans, mut at) = (Vec::new(), ByCopy::default());
        let mut declaring = Declaring::default();
        for sum in &sums {
            add_spans(&mut spans, &mut at, &sum.spans, |c| *c);
            declaring.list(&sum.declared, |c| *c);
        }
        (spans, declaring.done())
    }

    /// A small deterministic generator (no crate for it).
    struct Rng(u64);
    impl Rng {
        fn next(&mut self, n: u64) -> usize {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            (self.0 % n) as usize
        }
    }

    fn random_lists(rng: &mut Rng) -> Vec<Vec<Fake>> {
        let mut lists: Vec<Vec<Fake>> = (0..1 + rng.next(6))
            .map(|_| {
                (0..rng.next(40))
                    .map(|_| {
                        // Few copies and few places: repeats within and across lists.
                        let (copy, at, len) = (rng.next(5), rng.next(8) * 0x100, 0x40 * (1 + rng.next(4)));
                        match rng.next(3) {
                            0 => Fake::Refresh(copy, at, len),
                            1 => Fake::Write(copy, at, len),
                            _ => Fake::Other,
                        }
                    })
                    .collect()
            })
            .collect();
        // A submission may name a list twice.
        if rng.next(3) == 0 {
            let again = lists[rng.next(lists.len() as u64)].clone();
            lists.push(again);
        }
        lists
    }

    #[test]
    fn summaries_merged_give_what_the_walk_gives() {
        let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
        for _ in 0..2000 {
            let lists = random_lists(&mut rng);
            let (spans, declared) = merged(&lists);
            assert_eq!(spans, walked_spans(&lists), "{lists:?}");
            assert_eq!(declared, walked_declared(&lists), "{lists:?}");
        }
    }

    #[test]
    fn a_copy_whose_ranges_were_all_declared_before_gets_no_new_place() {
        // List 1 declares copy 1; list 2 declares copy 2 and copy 1 again
        // (all seen), then something new of copy 1: copy 1 keeps its first
        // place, copy 2 comes after it.
        let lists = vec![
            vec![Fake::Write(1, 0, 4)],
            vec![Fake::Write(2, 0, 4), Fake::Write(1, 0, 4), Fake::Write(1, 8, 4)],
        ];
        let (_, declared) = merged(&lists);
        assert_eq!(declared, vec![(1, vec![(0, 4), (8, 12)]), (2, vec![(0, 4)])]);
        assert_eq!(declared, walked_declared(&lists));
    }

    #[test]
    fn a_summary_knows_how_many_steps_it_was_made_of() {
        let steps = vec![Fake::Refresh(3, 0, 4), Fake::Other, Fake::Write(3, 0, 4)];
        let sum = Summary::of(steps.len(), seen(&steps), |c| *c);
        assert_eq!(sum.steps, 3);
        assert_eq!(sum.spans, vec![(3, 0, 4)]);
        assert_eq!(sum.declared, vec![(3, vec![(0, 4)])]);
    }

    /// A model of the submitting thread with flights: what a submission's
    /// preparing sees of the flights in the air. With the switch, as
    /// without it, submission N+1 is prepared only after N's flight (its
    /// declared ranges) is in the line, so N+1 never refreshes a range N
    /// may write as if nothing were writing it. The summary only changes
    /// how N+1's declared and refreshed ranges are found, not when the line
    /// is looked at.
    #[test]
    fn the_next_submission_sees_the_previous_flight_before_it_refreshes() {
        // (submission, refreshed (copy, from, to), declared written)
        struct Sub {
            lists: Vec<Vec<Fake>>,
        }
        #[derive(Default)]
        struct Line {
            /// Per copy, ranges flights may write.
            written: Vec<(Copy, usize, usize)>,
        }
        let mut rng = Rng(0xD1B5_4A32_D192_ED03);
        for _ in 0..500 {
            let subs: Vec<Sub> = (0..2 + rng.next(5)).map(|_| Sub { lists: random_lists(&mut rng) }).collect();
            let mut line = Line::default();
            // The thread: one submission at a time, under the lock; the
            // summaries were made before (at close).
            for sub in &subs {
                let (spans, declared) = merged(&sub.lists);
                // What it refreshes that a flight in the air may write: the
                // same with the walk, since the line holds every flight sent
                // before this one was taken.
                let stale = |spans: &[(Copy, usize, usize)]| -> Vec<(Copy, usize, usize)> {
                    spans
                        .iter()
                        .filter(|(c, f, t)| line.written.iter().any(|(wc, wf, wt)| wc == c && wf < t && f < wt))
                        .copied()
                        .collect()
                };
                assert_eq!(stale(&spans), stale(&walked_spans(&sub.lists)));
                // Every range of every earlier submission is in the line.
                for earlier in subs.iter().take_while(|s| !std::ptr::eq(*s, sub)) {
                    for (copy, ranges) in walked_declared(&earlier.lists) {
                        for (from, to) in ranges {
                            assert!(line.written.contains(&(copy, from, to)));
                        }
                    }
                }
                // Sent, then its flight in the line, before the lock goes.
                for (copy, ranges) in declared {
                    line.written.extend(ranges.into_iter().map(|(f, t)| (copy, f, t)));
                }
            }
        }
    }
}
