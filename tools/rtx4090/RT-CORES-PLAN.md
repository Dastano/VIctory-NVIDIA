# The title's ray tracing on the RTX 4090's RT cores: plan

Goal (the user's, 2026-11-28): the title's rays answered by the card's ray-tracing hardware (DXR inline ray tracing,
`RayQuery`, SM 6.5) instead of being emulated node by node, or left out (`VICTORY_BVH_BUILDER` off, the default:
rays find nothing and `crate::ambient` estimates the indirect light; shadows outside the sun's reach are near black).

## What the title does (docs/fork-ray-tracing.md)

No ray-tracing API: it builds and refits its own BVHs with 52 builder programs (XboxBvhBuilder), walks them in its
own compute loops (a stack in LDS per thread, or wave-wide stacks in scalars), and only the per-node test is
hardware (`image_bvh_intersect_ray` / `image_bvh64_intersect_ray`). NVIDIA has no per-node test: the RT cores only
take whole traversals, so the loop as a whole has to be replaced.

## What the code knows (file:line as of master 1a23d7a)

- Node pointers, box16/box32 and triangle nodes: `crates/victory-shader/src/bvh.rs:13-164` (5 vertices xyz f32,
  4 triangles; the byte at 60 says which vertex gets I and J). **No primitive id is defined**; spare vertex slots may
  hold the program's own data.
- Instances / headers: only in `docs/fork-ray-tracing.md:68-76` (128-byte records: 3x4 matrix, id/mask, flags,
  root, bottom-level address in 256-byte units; header marks bottom level / compacted / rehydrated / packed).
- Static structures ship packed and are unpacked once (`memory.rs:895`); dynamic ones are built every frame and
  live only on the card (`memory.rs:44-50`).
- Emission of the node test: `emit/lower.rs:6231` -> `emit/intersect.rs:64` (`bvh_memory` :264, `bvh_address`
  :350, `bvh_boxes` :369, `bvh_triangle` :501). No traversal-loop recognition (generic `loop_branches`,
  `lower.rs:569`).
- Host: no DXR at all (no tier check, no acceleration structures). Emitter: DXIL 1.6 / SM 6.6, heap handles
  present, no RayQuery ops. Host helper shaders compile with FXC cs_5_1: DXC needed for HLSL RayQuery.
- Known trace programs: `CS_RTShadowTrace32HQ_TiledFarOnly`, `CS_LaserLightShadowRaytracing` (wave-wide stack),
  `CS_IrradianceCache_Shade`; support: `CS_ScreenProbes_ActivateRays`, `CS_ScreenProbe_RayMiss_*`, `CS_RayMiss_*`,
  `CS_RaytracingResolve`, `CS_SortRayTiles`. AMD timing: RaytraceVBuffer 7.9 ms, screen probes 6.4, irradiance
  cache 3.5, primary probes 2.3, shadows 1.6.

## Steps

0. Measure the existing emulation on the RTX 4090 (`VICTORY_BVH_BUILDER=1`, city save, standing and driving):
   fps, card time, the shadows in shots. The card is ~50% idle (CPU-bound), so it may fit.
1. Extraction pass (GPU): walk a frame's top level and bottom levels in the card's copy of title memory; write
   vertex buffers (first, I, J order so DXR barycentrics are the console's I/J), instance descs and side tables
   (DXR instance/primitive index -> console instance pointer and triangle node pointer + k).
2. Build DXR BLAS/TLAS (over-sized counts, unused slots disabled; static bottom levels (~3,000) cached by address).
3. Check before touching a title shader: an HLSL (DXC) pass that sends the same rays through RayQuery and through
   the emulated walk and compares the hits.
4. One program by hand: a shadow-mask trace first (hit/miss only, no ids, no shading): RayQuery in the emitter and
   a per-hash patch that replaces its loop between its head and exit blocks, giving the loop's outputs back.
5. Then the screen probes' diffuse trace (the black shadows), which needs the hit record (ids via the side tables).

## Risks

Alpha-tested geometry (the loop may test alpha: the test has to be done in the RayQuery loop); the primitive-id and
flag layout is unknown; the builder's own cost and its open overwrite bug (fork-ray-tracing.md:109) stay; every
program needs its own patch (loop shapes differ; 64-lane programs on 32-lane waves); building thousands of
structures with over-sized counts may cost; results differ slightly from the console's rounding.
