//! LDS accumulators a compute program sets up from one thread only.
//!
//! The water's tiled light (`water_shading:CS_TransmissionVolumetrics_Tiled`
//! 5dc3b25986622949 and `_Tiled_LocalLights` 6f225b641c5b0708, GTA VI 1.0.2.1) keeps a tile's
//! nearest and farthest pixel in two LDS dwords: the group's first thread stores their
//! starting values (`ds_write2_b32` of `0` and `-1`, under `if (all(tid == 0))`), then every
//! thread folds its key in (`ds_max_u32`, `ds_min_u32`) and reads the result back, with no
//! barrier between (one console wave a group). That first thread stores only if it is still
//! running: the program's per-pixel early returns (a pixel outside the screen, or not water)
//! come before it. Where the tile's corner pixel is a boat's, a shore's or a bridge's, the
//! atomics fold into whatever the LDS held before: on the console another group's tile (near
//! enough), on the host anything at all. The farthest pixel then names no thread of the
//! group, no thread writes the tile's column of the light volume, and the whole tile is dark.
//!
//! [`hoists`] finds such dwords; the lowering stores their starting values in every thread at
//! the program's start, then the group meets, before any other LDS access. That is exact:
//! the LDS a group starts with is undefined, the program's own store (where it runs) stores
//! the same value again before the dword's first use in its order, and a read or atomic that
//! would see the early store instead of a write of this group would have seen undefined
//! memory. `VICTORY_WAVE64_FIX=0` turns it off.

use std::collections::BTreeMap;

use crate::gfx10::{Fields, Operand};
use crate::ir::{EXEC, Inst, Program, Reg, Value};

/// Whether accumulator starting values are stored at the program's start ([`hoists`]): on
/// unless `VICTORY_WAVE64_FIX=0`.
pub fn wave64_fix() -> bool {
    std::env::var_os("VICTORY_WAVE64_FIX").is_none_or(|v| v != "0")
}

/// The value an LDS dword must start with for the atomic `name` to accumulate into it, `None`
/// for an atomic that is no such accumulation.
fn identity(name: &str) -> Option<u32> {
    Some(match name.replace("_rtn", "").as_str() {
        "ds_min_u32" | "ds_and_b32" => u32::MAX,
        "ds_max_u32" | "ds_add_u32" | "ds_or_b32" | "ds_xor_b32" => 0,
        "ds_min_i32" => 0x7FFF_FFFF,
        "ds_max_i32" => 0x8000_0000,
        _ => return None,
    })
}

/// The constant `x` holds where instruction `i` of `insts` reads it: set by a plain
/// `v_mov_b32` of a constant earlier in the block, with EXEC not written since (so in every
/// lane that runs `i`).
fn constant(insts: &[Inst], i: usize, x: Value) -> Option<u32> {
    let d = insts[..i].iter().rposition(|j| j.writes.iter().any(|w| w.1 == x))?;
    let Fields::Valu(f) = &insts[d].fields else {
        return None;
    };
    let plain = insts[d].name() == "v_mov_b32"
        && f.abs == 0
        && f.neg == 0
        && f.opsel == 0
        && !f.clamp
        && f.omod == 0
        && f.sdwa.is_none()
        && f.dpp.is_none()
        && insts[d + 1..i]
            .iter()
            .all(|j| j.write(Reg::S(EXEC)).is_none() && j.write(Reg::S(EXEC + 1)).is_none());
    if !plain {
        return None;
    }
    match f.src.first()? {
        Operand::Const(c) => crate::resource::inline(*c),
        Operand::Literal(k) => Some(*k),
        _ => None,
    }
}

/// The VGPR operand `o` of instruction `i` as a constant ([`constant`]).
fn operand(insts: &[Inst], i: usize, o: Option<&Operand>) -> Option<u32> {
    let &Operand::Vector { reg, dwords: 1 } = o? else {
        return None;
    };
    constant(insts, i, insts[i].read(Reg::V(reg))?)
}

/// The program's LDS dwords (below `dwords`) set up as accumulators, with their starting
/// values: dwords every store to which is a `ds_write_b32` or `ds_write2_b32` of one constant
/// at a constant address, with at least one atomic at a constant address accumulating into
/// them with that constant as its identity. Empty when any other LDS write or atomic is in
/// the program, or one whose address or data is not such a constant: what it touches is not
/// known.
pub(crate) fn hoists(p: &Program, dwords: u32) -> Vec<(u32, u32)> {
    let mut stores: BTreeMap<u32, Vec<u32>> = BTreeMap::new();
    let mut atomics: BTreeMap<u32, Vec<u32>> = BTreeMap::new();
    for block in &p.blocks {
        let insts = &block.insts;
        for (i, inst) in insts.iter().enumerate() {
            let Fields::Ds {
                addr,
                data,
                offset0,
                offset1,
                gds,
                ..
            } = &inst.fields
            else {
                continue;
            };
            let name = inst.name();
            // (lane exchanges, which touch no LDS, and reads)
            if name.starts_with("ds_swizzle")
                || name.contains("permute")
                || name.starts_with("ds_read")
                || name == "ds_nop"
            {
                continue;
            }
            if *gds {
                return Vec::new();
            }
            let Some(base) = operand(insts, i, addr.as_ref()) else {
                return Vec::new();
            };
            let dword = |bytes: u32| base.wrapping_add(bytes) / 4;
            match name {
                "ds_write_b32" | "ds_write2_b32" => {
                    let at: Vec<u32> = if name == "ds_write_b32" {
                        vec![u32::from(*offset0) | u32::from(*offset1) << 8]
                    } else {
                        vec![u32::from(*offset0) * 4, u32::from(*offset1) * 4]
                    };
                    for (n, bytes) in at.into_iter().enumerate() {
                        let Some(k) = operand(insts, i, data.get(n)) else {
                            return Vec::new();
                        };
                        stores.entry(dword(bytes)).or_default().push(k);
                    }
                }
                _ => {
                    let Some(k) = identity(name) else {
                        return Vec::new();
                    };
                    let bytes = u32::from(*offset0) | u32::from(*offset1) << 8;
                    atomics.entry(dword(bytes)).or_default().push(k);
                }
            }
        }
    }
    stores
        .into_iter()
        .filter_map(|(dword, values)| {
            let k = values[0];
            let ids = atomics.get(&dword)?;
            (dword < dwords && values.iter().all(|&v| v == k) && ids.iter().all(|&v| v == k)).then_some((dword, k))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::hoists;
    use crate::ir;

    /// The water's tile programs: thread 0 stores 0 and -1 to dwords 0 and 1, every thread
    /// takes the maximum into dword 0 and the minimum into dword 1, and reads both back.
    #[test]
    fn the_tile_programs_maximum_and_minimum_are_accumulators() {
        let words = [
            0x7E26_0280u32, // v_mov_b32 v19, 0
            0x7E28_02C1,    // v_mov_b32 v20, -1
            0xD838_0100,
            0x0014_1313, // ds_write2_b32 v19, v19, v20 offset1:1
            0x7E20_0280, // v_mov_b32 v16, 0
            0xD81C_0004,
            0x0000_0D10, // ds_min_u32 v16, v13 offset:4
            0xD820_0000,
            0x0000_0F10, // ds_max_u32 v16, v15
            0x7E2C_0280, // v_mov_b32 v22, 0
            0xD8DC_0100,
            0x1300_0016, // ds_read2_b32 v[19:20], v22 offset1:1
            0xBF81_0000, // s_endpgm
        ];
        let code: Vec<u8> = words.iter().flat_map(|w| w.to_le_bytes()).collect();
        let p = ir::build(&code, true, 32).expect("ir::build");
        assert_eq!(hoists(&p, 128), vec![(0, 0), (1, u32::MAX)]);
        // (dwords past the program's LDS are not its own)
        assert_eq!(hoists(&p, 1), vec![(0, 0)]);
    }

    /// A store whose data is not a constant leaves the program's LDS unknown: nothing is
    /// hoisted.
    #[test]
    fn a_store_of_a_lane_value_hoists_nothing() {
        let words = [
            0x7E26_0280u32, // v_mov_b32 v19, 0
            0xD838_0100,
            0x0000_0113, // ds_write2_b32 v19, v1, v0 offset1:1
            0xD81C_0004,
            0x0000_0D13, // ds_min_u32 v19, v13 offset:4
            0xBF81_0000, // s_endpgm
        ];
        let code: Vec<u8> = words.iter().flat_map(|w| w.to_le_bytes()).collect();
        let p = ir::build(&code, true, 32).expect("ir::build");
        assert!(hoists(&p, 128).is_empty());
    }
}
