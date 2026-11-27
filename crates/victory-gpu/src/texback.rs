//! Textures shaders write that the title's own code then reads on the CPU.
//! On the console the CPU and the GPU share memory, so what a compute pass
//! stores into a texture is in title memory for the CPU to read. Here it stays
//! in the texture's host copy. The title's ocean is one: its wave passes write
//! a 256x256 height map (R32_FLOAT) and a displacement map (R32G32_FLOAT), and
//! its CPU code samples both, which is what boats float on; it read memory
//! nothing had written (run rw1, `VICTORY_READ_WATCH=1`: about 30 places in
//! the title read those two kinds), and boats sank.
//!
//! For textures of the kinds in `VICTORY_TEXTURE_BACK` (comma-separated
//! `<width>x<height>f<format>`; unset or `1`: the four water formats found by
//! the upstream trace; `0`: none), a list in which shaders write one copies it
//! into a read-back buffer of ours when it closes, and once the list has run
//! the texels are tiled into the texture's title memory in the console's
//! layout, as the console's GPU would have left them.
//!
//! shortcut: by kind, not by texture: textures of those kinds the title never
//! reads are copied back too (about a megabyte per such list). The copy shows
//! what the list's last write left; a title read in between sees the frame
//! before, as it would before the console's GPU has run.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, OnceLock, RwLock, Weak};

use windows::Win32::Graphics::Direct3D12::*;

use crate::object::This;

/// The kinds copied back: (width, height, format).
fn kinds() -> &'static [(u64, u32, u32)] {
    static KINDS: OnceLock<Vec<(u64, u32, u32)>> = OnceLock::new();
    KINDS.get_or_init(|| {
        const DEFAULT: &str = "256x256f41,256x256f16,64x64f16,128x128f36";
        let list = match std::env::var("VICTORY_TEXTURE_BACK").as_deref() {
            Ok("1") | Err(_) => DEFAULT.to_owned(),
            Ok(spec) => spec.to_owned(),
        };
        let kinds: Vec<(u64, u32, u32)> = list
            .split(',')
            .filter_map(|k| {
                let (size, format) = k.trim().split_once('f')?;
                let (w, h) = size.split_once('x')?;
                Some((w.parse().ok()?, h.parse().ok()?, format.parse().ok()?))
            })
            .collect();
        if !kinds.is_empty() {
            tracing::info!(?kinds, "textures of these kinds that shaders write are copied back into title memory (VICTORY_TEXTURE_BACK)");
        }
        kinds
    })
}

/// Whether title texture `resource` is of a kind copied back.
pub(crate) fn wanted(resource: This) -> bool {
    let kinds = kinds();
    !kinds.is_empty()
        && crate::resource::texture_kind(resource).is_some_and(|k| kinds.contains(&k))
}

/// An outstanding write to a specific allocation. Mapping and resource
/// changes invalidate only the writes that could touch the replaced memory.
struct Ticket {
    owner: usize,
    at: usize,
    end: usize,
    valid: AtomicBool,
}

/// Landings ([`land`]) hold it shared, several at once on the landing
/// threads ([`wide`]); a change of a mapping or resource marks tickets under
/// it alone, so that none lands across that change.
static TICKETS: RwLock<Vec<Weak<Ticket>>> = RwLock::new(Vec::new());
static TRACKING: AtomicBool = AtomicBool::new(false);

fn track(owner: usize, at: usize, end: usize) -> Arc<Ticket> {
    let ticket = Arc::new(Ticket { owner, at, end, valid: AtomicBool::new(true) });
    let mut tickets = TICKETS.write().unwrap_or_else(|e| e.into_inner());
    tickets.retain(|weak| weak.strong_count() != 0);
    tickets.push(Arc::downgrade(&ticket));
    TRACKING.store(true, Ordering::Relaxed);
    ticket
}

fn invalidate(matches: impl Fn(&Ticket) -> bool) {
    if !TRACKING.load(Ordering::Relaxed) {
        return;
    }
    TICKETS.write().unwrap_or_else(|e| e.into_inner()).retain(|weak| {
        let Some(ticket) = weak.upgrade() else { return false };
        if matches(&ticket) {
            ticket.valid.store(false, Ordering::Release);
        }
        true
    });
}

pub(crate) fn released(owner: usize) {
    invalidate(|ticket| ticket.owner == owner);
}

pub(crate) fn replaced(at: usize) {
    invalidate(|ticket| ticket.at == at);
}

pub(crate) fn remapped(at: usize, len: usize) {
    let end = at.saturating_add(len);
    invalidate(|ticket| at < ticket.end && ticket.at < end);
}

pub(crate) struct Target {
    pub(crate) copy: Arc<crate::resource::HostTexture>,
    layout: victory_formats::texture::Layout,
    ticket: Arc<Ticket>,
}

impl Target {
    pub(crate) fn new(resource: This) -> Option<Self> {
        // Keep the host copy independently of the title's resource lifetime.
        let copy = crate::resource::host_copy(resource)?;
        let _mapping = crate::watch::title_memory_reading();
        let (layout, at, size) = crate::resource::title_texture(resource)?;
        let end = at.checked_add(size).filter(|&end| at != 0 && end > at)?;
        let ticket = track(resource as usize, at, end);
        Some(Self { copy, layout, ticket })
    }

    pub(crate) fn owner(&self) -> usize { self.ticket.owner }
}

/// The readback resources and the allocation the pixels belong to.
pub(crate) struct Back {
    buffer: ID3D12Resource,
    footprints: Vec<D3D12_PLACED_SUBRESOURCE_FOOTPRINT>,
    rows: Vec<u32>,
    mips: u32,
    layout: victory_formats::texture::Layout,
    ticket: Arc<Ticket>,
}

impl Back {
    pub(crate) fn range(&self) -> (usize, usize) {
        (self.ticket.at, self.ticket.end)
    }
}

/// Records a host copy from a texture already in COPY_SOURCE.
pub(crate) fn record(list: &ID3D12GraphicsCommandList, target: Target) -> Option<Back> {
    if !target.ticket.valid.load(Ordering::Acquire) {
        return None;
    }
    let host = crate::host::host()?;
    let texture = &target.copy.texture;
    // SAFETY: a live texture kept through the command list's state tracking.
    let desc = unsafe { texture.GetDesc() };
    let mips = u32::from(desc.MipLevels.max(1));
    let subresources = mips * u32::from(desc.DepthOrArraySize.max(1));
    let n = subresources as usize;
    let mut footprints = vec![D3D12_PLACED_SUBRESOURCE_FOOTPRINT::default(); n];
    let mut rows = vec![0u32; n];
    let mut total = 0u64;
    // SAFETY: out arrays of `n` entries.
    unsafe {
        host.device.GetCopyableFootprints(
            &desc,
            0,
            subresources,
            0,
            Some(footprints.as_mut_ptr()),
            Some(rows.as_mut_ptr()),
            None,
            Some(&mut total),
        )
    };
    // The last footprint's advertised extent can omit row-end padding.
    // The tiler reads a pitched slice, so the mapped buffer must include it.
    for (footprint, &rows) in footprints.iter().zip(&rows) {
        let format = footprint.Footprint;
        let pitched = u64::from(format.RowPitch)
            .checked_mul(u64::from(rows))?
            .checked_mul(u64::from(format.Depth))?;
        total = total.max(footprint.Offset.checked_add(pitched)?);
    }
    let buffer = host.read_back_buffer(total)?;
    for (i, footprint) in footprints.iter().enumerate() {
        let to = D3D12_TEXTURE_COPY_LOCATION {
            // SAFETY: a borrowed pointer the location never releases.
            pResource: unsafe { std::mem::transmute_copy(&buffer) },
            Type: D3D12_TEXTURE_COPY_TYPE_PLACED_FOOTPRINT,
            Anonymous: D3D12_TEXTURE_COPY_LOCATION_0 { PlacedFootprint: *footprint },
        };
        let from = D3D12_TEXTURE_COPY_LOCATION {
            // SAFETY: as above.
            pResource: unsafe { std::mem::transmute_copy(texture) },
            Type: D3D12_TEXTURE_COPY_TYPE_SUBRESOURCE_INDEX,
            Anonymous: D3D12_TEXTURE_COPY_LOCATION_0 { SubresourceIndex: i as u32 },
        };
        // SAFETY: an open list; the texture is in the copy-source state.
        unsafe { list.CopyTextureRegion(&to, 0, 0, 0, &from, None) };
    }
    Some(Back { buffer, footprints, rows, mips, layout: target.layout, ticket: target.ticket })
}

/// Once the list has run, tile its texels back into the same title allocation.
pub(crate) fn land(back: &Back) {
    static COPIED: AtomicU64 = AtomicU64::new(0);
    static STALE: AtomicU64 = AtomicU64::new(0);
    let _mapping = crate::watch::title_memory_reading();
    let _ticket_guard = TICKETS.read().unwrap_or_else(|e| e.into_inner());
    if !back.ticket.valid.load(Ordering::Acquire) {
        let skipped = STALE.fetch_add(1, Ordering::Relaxed) + 1;
        if skipped.is_power_of_two() {
            tracing::info!(skipped, "skipping texture readbacks whose title allocation changed");
        }
        return;
    }
    let (at, end) = back.range();
    let size = end - at;
    let mut tiled = vec![0u8; size];
    // Keep title bytes the subresources do not cover.
    if !crate::host::read_into(at as *const u8, &mut tiled) {
        return;
    }
    let mut data = std::ptr::null_mut();
    // SAFETY: our read-back buffer, written by the list that has run.
    unsafe {
        if back.buffer.Map(0, None, Some(&mut data)).is_err() || data.is_null() {
            return;
        }
    }
    for (i, (footprint, &rows)) in back.footprints.iter().zip(&back.rows).enumerate() {
        let (mip, slice) = (i as u32 % back.mips, i as u32 / back.mips);
        let f = footprint.Footprint;
        let pitch = f.RowPitch as usize;
        let len = pitch * rows as usize * f.Depth as usize;
        // SAFETY: the subresource's part of the mapped buffer.
        let linear = unsafe { std::slice::from_raw_parts(data.cast::<u8>().add(footprint.Offset as usize), len) };
        if let Err(e) = back.layout.tile(&mut tiled, mip, slice, linear, pitch, pitch * rows as usize) {
            static ONCE: std::sync::Once = std::sync::Once::new();
            ONCE.call_once(|| tracing::warn!(?e, mip, slice, "a texture copied back could not be tiled"));
            // SAFETY: mapped above.
            unsafe { back.buffer.Unmap(0, None) };
            return;
        }
    }
    // SAFETY: mapped above.
    unsafe { back.buffer.Unmap(0, None) };
    crate::host::copy_mapped(at as *mut u8, tiled.as_ptr(), size, true);
    let copied = COPIED.fetch_add(1, Ordering::Relaxed) + 1;
    if copied == 1 || copied.is_multiple_of(128) {
        tracing::info!(copied, bytes = size, "texture readback landed");
    }
}


/// Whether the relay lands a flight's texture read-backs on the landing
/// threads, several at once (`VICTORY_RELAY_TEX_WIDE`; on, `=0`: one after
/// the other on the thread that copies back, as before).
///
/// The ocean's passes write some thirteen textures of the kinds copied back
/// a frame when the boat drives fast, mostly 256x256 floats; each landing
/// reads the texture's title memory, tiles the texels into it and writes it
/// through the kernel, under the lock against changes of mappings, all of
/// it on the one thread that lands flights (runs r96, r98: the relay's
/// "read-backs and lists" 4.5 ms a frame in the seconds under 29.5 fps,
/// 12.8 texture read-backs a frame, against 1.1-1.9 ms and 1.5-3.9 in the
/// fast ones: about 0.3 ms for each).
///
/// What lands is the same: the read-backs of textures whose title memory
/// overlaps are landed one after the other in the lists' order on one
/// thread ([`groups`]), the others touch no byte of each other's; all of a
/// flight's are landed before what follows its read-backs (the plan done
/// with, its bottom-of-pipe writes, fence signals), as before, and its
/// buffer read-backs after them, in their order (when one of those overlaps
/// a texture's memory, the flight's read-backs are landed one after the
/// other as before).
pub(crate) fn wide() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("VICTORY_RELAY_TEX_WIDE").is_none_or(|v| v != "0"))
}

/// The read-backs of `ranges` (title memory `[at, end)`, in the order they
/// land one after the other) in groups that share no byte with each other:
/// those whose memory overlaps, directly or through others, in one group, in
/// the order given.
pub(crate) fn groups(ranges: &[(usize, usize)]) -> Vec<Vec<usize>> {
    let mut order: Vec<usize> = (0..ranges.len()).collect();
    order.sort_by_key(|&i| ranges[i].0);
    let mut groups: Vec<Vec<usize>> = Vec::new();
    let mut end = 0;
    for i in order {
        let (at, to) = ranges[i];
        match groups.last_mut() {
            Some(group) if at < end => {
                group.push(i);
                end = end.max(to);
            }
            _ => {
                groups.push(vec![i]);
                end = to;
            }
        }
    }
    for group in &mut groups {
        group.sort_unstable();
    }
    groups
}

/// Lands `backs` ([`land`]), given in the order they land one after the
/// other: the groups of [`groups`] on the landing threads at once.
pub(crate) fn land_wide(backs: &[&Back]) {
    struct Shared<'a>(&'a [&'a Back]);
    impl Shared<'_> {
        fn get(&self, i: usize) -> &Back {
            self.0[i]
        }
    }
    // SAFETY: a back is only read (its buffer mapped and unmapped, which
    // D3D12 allows on any thread, by one thread: each back is in one
    // group); its ticket is atomic.
    unsafe impl Sync for Shared<'_> {}
    let ranges: Vec<(usize, usize)> = backs.iter().map(|b| b.range()).collect();
    let groups = groups(&ranges);
    let shared = Shared(backs);
    crate::land::each(groups.len(), &|g| {
        for &i in &groups[g] {
            land(shared.get(i));
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_readbacks_for_changed_allocations_are_invalidated() {
        let first = track(91, 0x1000, 0x2000);
        let next = track(92, 0x2000, 0x3000);
        let far = track(93, 0x4000, 0x5000);

        remapped(0x0f00, 0x200);
        assert!(!first.valid.load(Ordering::Acquire));
        assert!(next.valid.load(Ordering::Acquire));
        replaced(0x2000);
        assert!(!next.valid.load(Ordering::Acquire));
        assert!(far.valid.load(Ordering::Acquire));

        released(93);
        assert!(!far.valid.load(Ordering::Acquire));
        assert!(track(94, 0x2000, 0x3000).valid.load(Ordering::Acquire));
    }

    /// Landing the groups at once, in any order, leaves memory as landing
    /// every read-back one after the other does: overlapping ones stay in
    /// one group in their order, and groups share no byte.
    #[test]
    fn wide_groups_land_the_same_bytes_as_one_after_the_other() {
        let mut seed = 0x9e37_79b9_7f4a_7c15u64;
        let mut next = move |n: u64| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed % n
        };
        for round in 0..500u32 {
            let count = 1 + next(16) as usize;
            let backs: Vec<(usize, usize, u8)> = (0..count)
                .map(|i| {
                    let at = next(4096) as usize;
                    let len = next(if round % 3 == 0 { 64 } else { 1024 }) as usize;
                    (at, at + len, (i as u8).wrapping_mul(37).wrapping_add(round as u8) | 1)
                })
                .collect();
            // As a landing: what is there read, the texels put over a part
            // of it, the whole written back.
            let land = |memory: &mut [u8], (at, end, value): (usize, usize, u8)| {
                let mut tiled = memory[at..end].to_vec();
                for (k, byte) in tiled.iter_mut().enumerate() {
                    if k % 5 != 4 {
                        *byte = value.wrapping_add(k as u8);
                    }
                }
                memory[at..end].copy_from_slice(&tiled);
            };
            let mut one_after_the_other = vec![0u8; 8192];
            for &back in &backs {
                land(&mut one_after_the_other, back);
            }
            let ranges: Vec<(usize, usize)> = backs.iter().map(|b| (b.0, b.1)).collect();
            let groups = groups(&ranges);
            let mut all: Vec<usize> = groups.iter().flatten().copied().collect();
            all.sort_unstable();
            assert_eq!(all, (0..count).collect::<Vec<_>>(), "every read-back once");
            for (a, first) in groups.iter().enumerate() {
                assert!(first.windows(2).all(|w| w[0] < w[1]), "a group keeps the order given");
                for second in &groups[a + 1..] {
                    for &i in first {
                        for &j in second {
                            let (x, y) = (ranges[i], ranges[j]);
                            assert!(x.1 <= y.0 || y.1 <= x.0 || x.0 == x.1 || y.0 == y.1, "groups share no byte");
                        }
                    }
                }
            }
            // The groups at once on threads of their own, each in memory
            // as it was before them, what each landed put together; and
            // the groups one after the other in reverse.
            let memory = std::sync::Mutex::new(vec![0u8; 8192]);
            std::thread::scope(|scope| {
                for group in &groups {
                    let (memory, backs, land) = (&memory, &backs, &land);
                    scope.spawn(move || {
                        let mut own = vec![0u8; 8192];
                        for &i in group {
                            land(&mut own, backs[i]);
                        }
                        let mut memory = memory.lock().unwrap();
                        for &i in group {
                            let (at, end, _) = backs[i];
                            memory[at..end].copy_from_slice(&own[at..end]);
                        }
                    });
                }
            });
            assert_eq!(memory.into_inner().unwrap(), one_after_the_other, "round {round}: {backs:?}");
            let mut reversed = vec![0u8; 8192];
            for group in groups.iter().rev() {
                for &i in group {
                    land(&mut reversed, backs[i]);
                }
            }
            assert_eq!(reversed, one_after_the_other, "round {round}: {backs:?}");
        }
    }
}
