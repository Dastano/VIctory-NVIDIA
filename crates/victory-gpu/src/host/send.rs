//! What goes up to copies in the card's memory goes up as it was when its
//! submission was prepared (`VICTORY_SEND_OWN`, on by default; `0`: as
//! before).
//!
//! The sends were copies out of the copies' CPU sides, which the card makes
//! when it comes to the submission. By then later submissions are prepared
//! (two frames of them with `VICTORY_RELAY`), and their refreshes have
//! written the same CPU sides: the earlier submission's shaders then read a
//! later one's bytes. For what the title's CPU writes that is as on the
//! console (it does not write what the card still reads). But the title's
//! own buffer copies are commands in its lists, and those are done by us
//! when a submission is prepared: "a value to a buffer, a draw that reads
//! it" in one submission and the same with another value in the next is
//! ordinary, and both draws read the second value.
//!
//! So each submission's bytes are taken out of the CPU sides into memory of
//! the submission's own when they are taken from the dirty lists, and the
//! card copies them from there. Runs of a few words, which a refresh that
//! sends only what changed makes many of, go as pairs of offset and value,
//! stored by one dispatch per copy; the others as a copy each.

use super::*;
use crate::shadow::Shadow;
use std::sync::Arc;

pub(super) fn own() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("VICTORY_SEND_OWN").is_none_or(|v| v != "0"))
}

/// Runs up to this many bytes go as pairs.
const SHORT: usize = 256;

/// Whether a run of `len` bytes at offset `at` of a copy goes as pairs.
fn short(at: usize, len: usize) -> bool {
    len <= SHORT && at % 4 == 0 && len % 4 == 0
}

/// Whether a run of `len` bytes at offset `at` of a copy in the card's memory
/// is no copy command of its own, but pairs of one dispatch.
pub(crate) fn goes_as_words(at: usize, len: usize) -> bool {
    own() && short(at, len)
}

/// Whether the title's bytes that go in under flights' writes
/// (`Shadow::merge_under_writes`) go in by whole words
/// (`VICTORY_UNDER_WORDS=0`: by the bytes that changed, as before).
///
/// By bytes nearly every run (a float's three low bytes) was a copy command
/// of its own, and a submission with more than 4,096 of them stood until the
/// flights had landed: 4 to 14 ms of a steady frame (session play-1125-0203:
/// "more changed runs than a submission sends up"), for some 20 KB. Whole
/// words are pairs of one dispatch, and are not counted.
///
/// A word is taken to be the title's or the shaders', not shared: where the
/// title changed a byte of a word of which a flight's shader writes another
/// byte, that byte is as it was before the flight. The landing rests on the
/// same ([`crate::shadow::exact_back`]).
pub(crate) fn words_under() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| own() && std::env::var_os("VICTORY_UNDER_WORDS").is_none_or(|v| v != "0"))
}

const SCATTER: &str = r#"
#define RS "RootConstants(num32BitConstants=2, b0), SRV(t0), UAV(u0)"
cbuffer Part : register(b0) { uint at; uint count; };
ByteAddressBuffer pairs : register(t0);
RWByteAddressBuffer card : register(u0);
[RootSignature(RS)]
[numthreads(64, 1, 1)]
void main(uint3 id : SV_DispatchThreadID) {
    if (id.x >= count) return;
    uint2 p = pairs.Load2(at + id.x * 8u);
    card.Store(p.x, p.y);
}
"#;

struct Pass {
    root: ID3D12RootSignature,
    pso: ID3D12PipelineState,
}

fn pass(host: &Host) -> Option<&'static Pass> {
    static PASS: OnceLock<Option<Pass>> = OnceLock::new();
    PASS.get_or_init(|| {
        let code = compile(SCATTER, windows::core::s!("main"), windows::core::s!("cs_5_1")).ok()?;
        // SAFETY: the code lives through the calls; the root signature is
        // inside it.
        unsafe {
            let root: ID3D12RootSignature = host.device.CreateRootSignature(0, &code).ok()?;
            let pso: ID3D12PipelineState = host
                .device
                .CreateComputePipelineState(&D3D12_COMPUTE_PIPELINE_STATE_DESC {
                    pRootSignature: std::mem::transmute_copy(&root),
                    CS: D3D12_SHADER_BYTECODE {
                        pShaderBytecode: code.as_ptr().cast(),
                        BytecodeLength: code.len(),
                    },
                    ..Default::default()
                })
                .ok()?;
            name(&pso, || "victory: changed words sent to the card".into());
            Some(Pass { root, pso })
        }
    })
    .as_ref()
}

/// Memory a submission's sends come from, mapped.
pub(super) struct Memory {
    buffer: ID3D12Resource,
    data: usize,
    size: u64,
}

// SAFETY: a D3D12 object (free-threaded) and its mapping, with one owner.
unsafe impl Send for Memory {}

/// What submissions that have run gave back.
static SPARE: Mutex<Vec<Memory>> = Mutex::new(Vec::new());

/// At most this much is kept (of the card's memory, with [`on_card`]).
fn kept() -> u64 {
    if on_card() { 256 << 20 } else { 768 << 20 }
}

/// Whether the memory lies in the card's memory, written by the CPU through
/// the bus (`VICTORY_SEND_BAR=1`): the copies on the card's drawing queue, which
/// nothing overlaps, then cross no bus. In a captured frame they took 5.4 ms of
/// the card's 30 (342 copies), and take 0.4 this way; but filling it takes the
/// submitting thread 8-9 ms a frame instead of 3 (73-93 MB, run card-3 against
/// card-1), so it is a loss while the card waits for that thread: off by
/// default.
fn on_card() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("VICTORY_SEND_BAR").is_some_and(|v| v == "1"))
}

fn memory(host: &Host, size: u64) -> Option<Memory> {
    {
        let mut spare = SPARE.lock().unwrap_or_else(|e| e.into_inner());
        let fit = spare.iter().enumerate().filter(|(_, m)| m.size >= size).min_by_key(|(_, m)| m.size).map(|(i, _)| i);
        if let Some(i) = fit {
            return Some(spare.swap_remove(i));
        }
    }
    // By doubles, so that the next of about this size finds it: what
    // submissions send differs from frame to frame.
    let size = size.max(8 << 20).next_power_of_two();
    let mapped = if on_card() { host.mapped_card_buffer(size, false) } else { None };
    let (buffer, data) = match mapped {
        Some(made) => made,
        None => host.upload_buffer(size)?,
    };
    name(&buffer, || "bytes a submission sends up".into());
    Some(Memory {
        buffer,
        data: data as usize,
        size,
    })
}

/// The submission has run.
pub(super) fn give_back(memory: Memory) {
    let mut spare = SPARE.lock().unwrap_or_else(|e| e.into_inner());
    spare.push(memory);
    spare.sort_by_key(|m| std::cmp::Reverse(m.size));
    while spare.iter().map(|m| m.size).sum::<u64>() > kept() {
        spare.remove(0);
    }
}

/// Since the last frame-rate line: bytes copied, runs copied, words stored,
/// microseconds of taking them out of the CPU sides.
pub(super) static STATS: [std::sync::atomic::AtomicU64; 4] = [const { std::sync::atomic::AtomicU64::new(0) }; 4];

/// One copy's sends: ranges copied (offset in the copy, in the memory,
/// bytes), and its pairs (where in the memory, how many).
#[derive(Debug, Clone, PartialEq, Eq)]
struct Copy {
    runs: Vec<(usize, u64, usize)>,
    pairs: (u64, u32),
}

/// Where a submission's bytes are taken to. Borrows the dirty lists it was
/// taken from: with [`beside`] their bytes are taken while it lives, and it
/// waits for that when dropped (so it is not to be forgotten).
pub(super) struct Own<'a> {
    memory: Memory,
    copies: Vec<Copy>,
    taking: Option<Beside>,
    _dirty: std::marker::PhantomData<&'a ()>,
}

/// Runs whose pairs are one piece of work.
const RUNS_AT_A_TIME: usize = 4096;

/// Whether a submission's bytes are taken out of the CPU sides on a thread
/// of their own while the submitting thread records the list that sends
/// them (`VICTORY_CC_TAKE_BESIDE`, on by default; `0`: on the submitting
/// thread, before the list, as before).
///
/// Of the 3.6 ms a frame of copies in the card's memory in run r74 (2,000
/// to 6,000 draws), 1.5 were the bytes taken (20-29 MB in some 400 runs and
/// 30,000-180,000 words) and 1.2 the list recorded and sent, one after the
/// other on the submitting thread; the list needs only where the bytes go
/// ([`layout`], still made first), not the bytes. The list goes to the
/// queue only once they are all taken ([`Own::wait`]). The same ranges go
/// to the same places in the memory with the same bytes: between the dirty
/// lists being taken and the list sent, the submitting thread records only,
/// and it alone writes the CPU sides in those ranges (refreshes; a landing
/// writes under shader writes, as it may while they are taken on it).
fn beside() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("VICTORY_CC_TAKE_BESIDE").is_none_or(|v| v != "0"))
}

/// Where each copy's sends lie in the memory, and the bytes of memory they
/// need, for the dirty ranges of each copy in turn.
fn layout<'r>(dirty: impl Iterator<Item = &'r [(usize, usize)]>) -> (Vec<Copy>, u64) {
    // Runs of whole words of the copy: only those are pairs. (A byte beside
    // them in a word may be the card's.)
    let short = |&(a, b): &(usize, usize)| short(a, b - a);
    let mut at = 0u64;
    let mut copies = Vec::new();
    for ranges in dirty {
        let mut runs = Vec::new();
        for &(a, b) in ranges.iter().filter(|r| !short(r)) {
            runs.push((a, at, b - a));
            at += ((b - a) as u64).next_multiple_of(16);
        }
        let words: usize = ranges.iter().filter(|r| short(r)).map(|r| (r.1 - r.0) / 4).sum();
        copies.push(Copy { runs, pairs: (at, words as u32) });
        at += (words as u64 * 8).next_multiple_of(16);
    }
    (copies, at)
}

/// The bytes of a submission to be taken: runs copied (from, to, bytes) and
/// pieces of pairs (the copy's CPU side, its ranges, where the pairs go).
struct Taking<'a> {
    moves: Vec<(usize, usize, usize)>,
    work: Vec<(usize, &'a [(usize, usize)], usize)>,
}

impl<'a> Taking<'a> {
    /// For `sources` (each copy's CPU side and dirty ranges) laid out as
    /// `copies` in memory mapped at `data`.
    fn new(sources: &[(usize, &'a [(usize, usize)])], copies: &[Copy], data: usize) -> Self {
        let short = |&(a, b): &(usize, usize)| short(a, b - a);
        let mut moves: Vec<(usize, usize, usize)> = Vec::new();
        let mut work: Vec<(usize, &'a [(usize, usize)], usize)> = Vec::new();
        for (&(from, ranges), copy) in sources.iter().zip(copies) {
            moves.extend(copy.runs.iter().map(|&(a, to, n)| (from + a, data + to as usize, n)));
            // (80,000 runs of some words a frame: in pieces of work.)
            let mut to = data + copy.pairs.0 as usize;
            for ranges in ranges.chunks(RUNS_AT_A_TIME) {
                let words: usize = ranges.iter().filter(|r| short(r)).map(|r| (r.1 - r.0) / 4).sum();
                if words > 0 {
                    work.push((from, ranges, to));
                    to += words * 8;
                }
            }
        }
        Taking { moves, work }
    }

    /// Copies the runs and stores the pairs.
    ///
    /// # Safety
    /// The sources' ranges lie in mappings no one writes meanwhile, and the
    /// memory at `data` holds the layout's bytes.
    unsafe fn run(self) {
        let Taking { moves, work } = self;
        let one = |i: usize| {
            let (from, ranges, to) = work[i];
            let mut to = to as *mut [u32; 2];
            for &(a, b) in ranges.iter().filter(|&&(a, b)| short(a, b - a)) {
                for offset in (a..b).step_by(4) {
                    // SAFETY: inside the copy's mapping (its CPU side, which
                    // the thread that asked, and waits, alone writes in these
                    // ranges) and inside the memory, which has room for every
                    // pair, a piece of work's behind the last one's.
                    unsafe {
                        let word = ((from + offset) as *const u32).read_unaligned();
                        to.write([offset as u32, word]);
                        to = to.add(1);
                    }
                }
            }
        };
        crate::land::each_under(work.len(), &one);
        crate::shadow::copy_all(moves);
    }
}

/// A [`Taking`] on the thread for it ([`beside`]): whether it is done, and
/// whether it panicked.
struct Beside(Arc<(Mutex<Option<bool>>, std::sync::Condvar)>);

impl Beside {
    /// Hands `taking` to the thread; done on this one (`None`) if there is
    /// no thread.
    ///
    /// # Safety
    /// As [`Taking::run`]; and what it borrows lives until the result is
    /// waited for ([`Beside::wait`], or dropped), so it is not forgotten.
    unsafe fn start(taking: Taking<'_>, began: std::time::Instant) -> Option<Beside> {
        type Job = Box<dyn FnOnce() + Send>;
        static THREAD: OnceLock<Option<Mutex<std::sync::mpsc::Sender<Job>>>> = OnceLock::new();
        let thread = THREAD.get_or_init(|| {
            let (send, jobs) = std::sync::mpsc::channel::<Job>();
            std::thread::Builder::new()
                .name("victory-take".into())
                .spawn(move || {
                    crate::cores::keep_helper_on_fast_cores("victory-take");
                    for job in jobs {
                        job();
                    }
                })
                .ok()
                .map(|_| Mutex::new(send))
        });
        let done = Arc::new((Mutex::new(None), std::sync::Condvar::new()));
        // SAFETY: the caller's: what it borrows outlives the waiting.
        let taking: Taking<'static> = unsafe { std::mem::transmute::<Taking<'_>, Taking<'static>>(taking) };
        let job: Job = {
            let done = done.clone();
            Box::new(move || {
                // SAFETY: the caller's.
                let ran = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| unsafe { taking.run() }));
                STATS[3].fetch_add(began.elapsed().as_micros() as u64, Ordering::Relaxed);
                *done.0.lock().unwrap_or_else(|e| e.into_inner()) = Some(ran.is_err());
                done.1.notify_all();
            })
        };
        let Some(thread) = thread else {
            job();
            return Some(Beside(done));
        };
        match thread.lock().unwrap_or_else(|e| e.into_inner()).send(job) {
            Ok(()) => Some(Beside(done)),
            // (The thread is gone: the job comes back, done here.)
            Err(std::sync::mpsc::SendError(job)) => {
                job();
                Some(Beside(done))
            }
        }
    }

    /// Waits for the bytes; whether they were all taken.
    fn wait(&self) -> bool {
        let mut done = self.0.0.lock().unwrap_or_else(|e| e.into_inner());
        while done.is_none() {
            done = self.0.1.wait(done).unwrap_or_else(|e| e.into_inner());
        }
        *done == Some(false)
    }
}

impl Drop for Beside {
    fn drop(&mut self) {
        self.wait();
    }
}

/// Takes what `dirty` sends up out of the CPU sides (with [`beside`] on a
/// thread of its own: [`Own::wait`] before the list that sends it goes to
/// the queue).
pub(super) fn take<'a>(host: &Host, dirty: &'a [(std::sync::Arc<Shadow>, Vec<(usize, usize)>)]) -> Option<Own<'a>> {
    let began = std::time::Instant::now();
    pass(host)?;
    let (copies, at) = layout(dirty.iter().map(|(_, ranges)| ranges.as_slice()));
    if at == 0 {
        return None;
    }
    let memory = memory(host, at)?;
    let sources: Vec<(usize, &'a [(usize, usize)])> =
        dirty.iter().map(|(shadow, ranges)| (shadow.card_data() as usize, ranges.as_slice())).collect();
    let taking = Taking::new(&sources, &copies, memory.data);
    STATS[0].fetch_add(taking.moves.iter().map(|m| m.2 as u64).sum(), Ordering::Relaxed);
    STATS[1].fetch_add(taking.moves.len() as u64, Ordering::Relaxed);
    STATS[2].fetch_add(copies.iter().map(|c| u64::from(c.pairs.1)).sum(), Ordering::Relaxed);
    let taking = if beside() {
        // SAFETY: the ranges lie in the copies' mappings, written meanwhile
        // by no one ([`beside`]); the memory holds the layout; `dirty` is
        // borrowed by the `Own`, which waits when dropped.
        unsafe { Beside::start(taking, began) }
    } else {
        // SAFETY: as above, on this thread.
        unsafe { taking.run() };
        STATS[3].fetch_add(began.elapsed().as_micros() as u64, Ordering::Relaxed);
        None
    };
    Some(Own { memory, copies, taking, _dirty: std::marker::PhantomData })
}

impl Own<'_> {
    /// Waits until the bytes are all in the memory ([`beside`]): before the
    /// list that sends them goes to the queue.
    pub(super) fn wait(&self) {
        if self.taking.as_ref().is_some_and(|t| !t.wait()) {
            fail(format_args!("the bytes a submission sends up could not be taken out of the CPU sides"));
        }
    }

    /// Records the sends into `list`; the copies that `looked` says are in
    /// `COPY_DEST`, the others in the common state. Those with a "before" are
    /// left readable by compute shaders.
    ///
    /// # Safety
    /// An open list of ours, whose compute state is set anew behind this.
    pub(super) unsafe fn record(
        &self,
        host: &Host,
        list: &ID3D12GraphicsCommandList,
        dirty: &[(std::sync::Arc<Shadow>, Vec<(usize, usize)>)],
        looked: impl Fn(&Shadow) -> bool,
    ) {
        let read = D3D12_RESOURCE_STATE_NON_PIXEL_SHADER_RESOURCE;
        let write = D3D12_RESOURCE_STATE_UNORDERED_ACCESS;
        let dest = D3D12_RESOURCE_STATE_COPY_DEST;
        let (mut first, mut last) = (Vec::new(), Vec::new());
        for ((shadow, _), copy) in dirty.iter().zip(&self.copies) {
            for &(a, from, n) in &copy.runs {
                // SAFETY: the caller's; live buffers, ranges inside them.
                unsafe { list.CopyBufferRegion(&shadow.buffer, a as u64, &self.memory.buffer, from, n as u64) };
            }
            let mut state = (looked(shadow) || !copy.runs.is_empty()).then_some(dest);
            if copy.pairs.1 > 0 {
                if let Some(was) = state {
                    first.push(transition(&shadow.buffer, was, write));
                }
                state = Some(write);
            }
            let hashed = shadow.card_sides().is_some_and(|(_, before, _)| before.is_some());
            if let Some(was) = state
                && hashed
                && (!copy.runs.is_empty() || copy.pairs.1 > 0)
            {
                last.push(transition(&shadow.buffer, was, read));
            }
        }
        // SAFETY: the caller's; live objects.
        unsafe {
            if !first.is_empty() {
                list.ResourceBarrier(&first);
            }
            if let Some(pass) = pass(host)
                && self.copies.iter().any(|c| c.pairs.1 > 0)
            {
                list.SetComputeRootSignature(&pass.root);
                list.SetPipelineState(&pass.pso);
                list.SetComputeRootShaderResourceView(1, self.memory.buffer.GetGPUVirtualAddress());
                for ((shadow, _), copy) in dirty.iter().zip(&self.copies).filter(|(_, c)| c.pairs.1 > 0) {
                    list.SetComputeRootUnorderedAccessView(2, shadow.buffer.GetGPUVirtualAddress());
                    const MOST: u32 = 64 * D3D12_CS_DISPATCH_MAX_THREAD_GROUPS_PER_DIMENSION;
                    let (at, words) = copy.pairs;
                    let mut done = 0;
                    while done < words {
                        let n = (words - done).min(MOST);
                        let constants = [at as u32 + done * 8, n];
                        list.SetComputeRoot32BitConstants(0, 2, constants.as_ptr().cast(), 0);
                        list.Dispatch(n.div_ceil(64), 1, 1);
                        done += n;
                    }
                }
            }
            if !last.is_empty() {
                list.ResourceBarrier(&last);
            }
        }
    }

    /// The memory, to be kept until the submission has run.
    pub(super) fn memory(self) -> Memory {
        self.wait();
        let Own { memory, taking, .. } = self;
        drop(taking);
        memory
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Dirty ranges of a copy of `len` bytes: in order, apart, of every
    /// kind (runs of words, of odd bytes, short and long).
    fn ranges(seed: &mut u64, len: usize) -> Vec<(usize, usize)> {
        let mut next = |n: usize| {
            *seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            ((*seed >> 33) as usize) % n
        };
        let mut out = Vec::new();
        let mut at = next(64);
        while at < len {
            let n = match next(4) {
                0 => 4 * (1 + next(64)),
                1 => 1 + next(300),
                2 => 4 * (1 + next(16)),
                _ => 1 + next(20_000),
            };
            let a = if next(2) == 0 { at.next_multiple_of(4) } else { at };
            let b = (a + n).min(len);
            if a < b {
                out.push((a, b));
            }
            at = b + 1 + next(512);
        }
        out
    }

    /// The bytes taken, laid out by [`layout`], with or without [`Beside`].
    fn taken(sources: &[(Vec<u8>, Vec<(usize, usize)>)], beside: bool) -> (Vec<Copy>, Vec<u8>) {
        let (copies, at) = layout(sources.iter().map(|(_, r)| r.as_slice()));
        let mut memory = vec![0u64; (at as usize).div_ceil(8)];
        let data = memory.as_mut_ptr() as usize;
        let borrowed: Vec<(usize, &[(usize, usize)])> = sources.iter().map(|(b, r)| (b.as_ptr() as usize, r.as_slice())).collect();
        let taking = Taking::new(&borrowed, &copies, data);
        // SAFETY: the sources and the memory live through the waiting, and
        // nothing writes them meanwhile.
        unsafe {
            if beside {
                assert!(Beside::start(taking, std::time::Instant::now()).is_some_and(|t| t.wait()));
            } else {
                taking.run();
            }
        }
        let bytes = memory.iter().flat_map(|w| w.to_le_bytes()).take(at as usize).collect();
        (copies, bytes)
    }

    #[test]
    fn beside_takes_the_same_bytes_to_the_same_places() {
        let mut seed = 7u64;
        let mut sources: Vec<(Vec<u8>, Vec<(usize, usize)>)> = Vec::new();
        for (i, len) in [1 << 16, 3 << 20, 100, 6 << 20, 0, 1 << 20].into_iter().enumerate() {
            let bytes: Vec<u8> = (0..len).map(|j| (j * 31 + i * 7 + j / 977) as u8).collect();
            let mut r = ranges(&mut seed, len);
            if i == 3 {
                // More than [`crate::shadow::copy_all`] does on one thread.
                r = vec![(8, 5 << 20), (5 << 20 | 4, 5 << 20 | 64)];
            }
            sources.push((bytes, r));
        }
        let (copies, here) = taken(&sources, false);
        for _ in 0..3 {
            let (laid, there) = taken(&sources, true);
            assert_eq!(laid, copies, "the same ranges to the same places");
            assert!(here == there, "the same bytes");
        }
        // And they are the copies' bytes: each run where the layout says,
        // the words of the short runs as pairs in order.
        for ((bytes, ranges), copy) in sources.iter().zip(&copies) {
            for &(a, to, n) in &copy.runs {
                assert_eq!(&here[to as usize..][..n], &bytes[a..a + n]);
            }
            let words: Vec<(u32, u32)> = ranges
                .iter()
                .filter(|&&(a, b)| short(a, b - a))
                .flat_map(|&(a, b)| (a..b).step_by(4))
                .map(|o| (o as u32, u32::from_le_bytes(bytes[o..o + 4].try_into().unwrap())))
                .collect();
            assert_eq!(words.len(), copy.pairs.1 as usize);
            for (i, &(o, w)) in words.iter().enumerate() {
                let at = copy.pairs.0 as usize + i * 8;
                assert_eq!(u32::from_le_bytes(here[at..at + 4].try_into().unwrap()), o);
                assert_eq!(u32::from_le_bytes(here[at + 4..at + 8].try_into().unwrap()), w);
            }
        }
        assert!(copies.iter().any(|c| c.pairs.1 > 0) && copies.iter().any(|c| !c.runs.is_empty()));
    }
}
