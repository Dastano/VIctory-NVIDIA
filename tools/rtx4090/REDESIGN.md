# Redesign plan: less per-frame refresh and copy work (2026-11-27)

Goal: take >= 8 ms off the submitting thread in heavy frames (boat drive: 28-30 ms of work, dips under 30 fps in
20-40 % of seconds). Local optimisations are exhausted (NOTES-master.md, r35-r109). Stages, ranked by gain/effort,
each behind a switch and judged by the frame line's split plus >= 3 boat runs a side (the route spreads ~2 fps).

0. Per-copy counters: refreshes asked / copying something, frames since the title last wrote it, frames since a
   relay compare found shader writes, fills reading it; the 20 busiest copies on the frame line; fill source ranges.
1. Frozen copies (`VICTORY_FREEZE`): a copy with no title or shader write for N frames is frozen, its pages
   guarded read-only; lists push no Step::Refresh for it, only stamp it; the first write faults, thaws it and
   queues a full refresh of its span before the next submission. Thawed twice in 120 frames: banned. Expected
   4.5-5.5 ms (most of ~6,000 no-op refreshes and a share of the write-watch askings).
2. Constant ring as CPU-only (`VICTORY_RING_CPU`): learn its extent from fill sources, confirm the GPU never
   writes it; fills from it skip card-step planning and flight waits; the heap's card sends leave it out. ~4 ms.
3. Copies shaders never write kept only in card memory over ReBAR (generalise `wc_bar`). 1-2 ms.
4. Landing on demand for ranges the title never reads (riskiest; last).
