//! Fortress lock / water-gap placement.
//!
//! Per section (in BFS-rank order): score every lockable tile by how much of
//! the map it walls off, enforce two hard rules (the section's own fort stays
//! reachable; no earlier fort is stranded), then pick with two choice-first
//! refinements:
//!
//! - **Gate guarantee** — the section named by `gate_section` (the shaping
//!   pass's random fort pick) hard-filters to candidates that make the goal
//!   unreachable, so some fort actually gates the goal whenever the geometry
//!   offers a severing tile. Infeasible sections carry the duty forward.
//! - **Choice guard** — a lock that would shrink the world's in-band route
//!   count (measured by `analyze_route_choice`) is demoted and the next-best
//!   candidate tried, bounded by `choice_guard_tries`. This replaces the
//!   archetype-era `blocks_later_fort_bonus` chain magnet (now defaulted 0).

use super::*;

use super::knobs::LockScoring;
use super::route_choice::{DEFAULT_SLACK, analyze_route_choice};
use super::types::{BuildResult, BuiltWorld, LockAssignment, SlotAssignment, SlotKind, stamp_slots};

/// The five always-on W8 screen-3 bridges (see `qol::overworld_map`'s
/// `W8_BRIDGE_EDITS`). Locking one gaps it out as water until its fortress is
/// beaten — a deliberate showcase, so the lock scorer nudges toward them.
const W8_BRIDGE_LOCK_POSITIONS: &[(usize, usize)] = &[(5, 51), (5, 53), (5, 55), (5, 57), (5, 59)];

/// A scored lock candidate tracked during selection.
#[derive(Clone, Copy)]
struct ScoredLock {
    pos: Pos,
    gap_tile: u8,
    replace_tile: u8,
    score: i32,
    safe: bool,
    blocks_target: bool,
}

// Reason: every argument represents a distinct, independent input to lock
// placement (geometry, slot list, count, knobs, gate hint, safety flag, RNG).
// No subset clusters into a meaningful concept — bundling would be a clippy
// bandage, not a real abstraction.
#[allow(clippy::too_many_arguments)]
pub(super) fn place_locks<R: Rng>(
    grid: &Grid,
    pipe_pairs: &[TeleportEdge],
    start_pos: Option<(usize, usize)>,
    target_pos: Option<(usize, usize)>,
    slots: &[SlotAssignment],
    fort_count: usize,
    knobs: &LockScoring,
    gate_section: Option<usize>,
    force_safe: bool,
    world_idx: usize,
    rng: &mut R,
) -> Vec<LockAssignment> {
    let mut locks: Vec<LockAssignment> = Vec::new();
    let mut locked_tiles: HashSet<(usize, usize)> = HashSet::new();

    // The section whose lock still owes the goal a gate. Starts at the
    // shaping pass's hint and carries forward when a section's geometry
    // can't sever the goal.
    let mut gate_pending = gate_section;

    // Choice-guard measurement: in-band route count with a given lock list.
    // Locks are modeled as conditional edges by the scorer, so the raw build
    // grid is correct here.
    let measure_routes = |trial_locks: Vec<LockAssignment>| -> usize {
        let world = BuiltWorld {
            world_idx,
            grid: grid.clone(),
            slots: slots.to_vec(),
            locks: trial_locks,
            section_count: fort_count,
            pipe_pairs: pipe_pairs.to_vec(),
            hb_sprites: Vec::new(),
        };
        analyze_route_choice(&world, DEFAULT_SLACK).routes.len()
    };

    // Build a base grid with forts/levels stamped so BFS sees them as nodes.
    // This grid does NOT have any locks on it.
    let mut base_grid = grid.clone();
    stamp_slots(&mut base_grid, slots);

    // Process each fortress in section order
    for section_idx in 0..fort_count {
        let fort_pos = match slots
            .iter()
            .find(|s| s.section == section_idx && s.kind == SlotKind::Fortress)
        {
            Some(s) => s.pos,
            None => continue,
        };

        // Build the "current state" grid: base grid + all previously placed locks
        // + all locks from earlier sections opened (simulating progression).
        // When checking section N's lock, sections 0..N-1 have been beaten,
        // so their locks are open. The new lock we're testing is the only closed one.
        let build_test_grid = |new_lock: Option<((usize, usize), u8)>| -> Grid {
            let mut g = base_grid.clone();
            // Place all previously committed locks
            for prev in &locks {
                if prev.fort_section < section_idx {
                    // Earlier section — fort beaten, lock opened (restore path tile)
                    g.set(prev.pos.0, prev.pos.1, prev.replace_tile);
                } else {
                    // Same or later section — lock still closed
                    g.set(prev.pos.0, prev.pos.1, prev.gap_tile);
                }
            }
            // Place the candidate lock
            if let Some((pos, gap)) = new_lock {
                g.set(pos.0, pos.1, gap);
            }
            g
        };

        // Find all lockable path tiles not yet used
        let reference_grid = build_test_grid(None);
        let mut candidates: Vec<(usize, usize)> = Vec::new();
        for r in 0..reference_grid.rows() {
            for c in 0..reference_grid.cols {
                let tile = reference_grid.get(r, c);
                if LOCKABLE_TILES.contains(&tile) && !locked_tiles.contains(&(r, c)) {
                    // Row 7 and row 8 share Map_Completions bit ($01).
                    // A lock/bridge/gap is completion-unsafe — it would
                    // prevent the fallthrough between rows 7 and 8.
                    // Skip if the paired row has a completable slot.
                    if r == 7 || r == 8 {
                        let paired_row = if r == 7 { 8 } else { 7 };
                        let pair_completable = slots.iter().any(|s| {
                            s.pos == (paired_row, c)
                                && matches!(s.kind, SlotKind::Level | SlotKind::Fortress | SlotKind::Pipe | SlotKind::BonusGame | SlotKind::ToadHouse)
                        });
                        if pair_completable {
                            continue;
                        }
                    }
                    candidates.push((r, c));
                }
            }
        }

        candidates.shuffle(rng);

        // All hard-rule-passing candidates, scored. Selection happens after
        // the loop (gate filter → safe preference → choice guard).
        let mut scored: Vec<ScoredLock> = Vec::new();

        // Open grid (no candidate lock) is constant for all candidates in this
        // section — hoist the BFS to avoid redundant walks per candidate.
        let open_grid = build_test_grid(None);
        let open_node_count = walk_map(&open_grid, pipe_pairs, start_pos, world_idx).nodes.len() as i32;

        // If a previous lock in this world already blocks the target, suppress
        // the target-blocking bonus to avoid stacking multiple locks against
        // the airship/Bowser.
        let target_already_locked = locks.iter().any(|l| l.blocks_target);

        for &cand_pos in &candidates {
            let tile = reference_grid.get(cand_pos.0, cand_pos.1);
            let gap = gap_tile_for(tile);

            // Hard rule 1: with this lock placed (and earlier locks opened),
            // the current fortress must still be reachable from start.
            let test_grid = build_test_grid(Some((cand_pos, gap)));
            let walk = walk_map(&test_grid, pipe_pairs, start_pos, world_idx);

            if !walk.nodes.contains(&fort_pos) {
                continue;
            }

            // Hard rule 2: this lock must not block any earlier fortress.
            // Check each earlier section's fort is reachable when its own
            // lock (and all locks before it) are open but this new lock is closed.
            let blocks_earlier = locks.iter().any(|prev_lock| {
                let prev_fort = slots.iter()
                    .find(|s| s.section == prev_lock.fort_section && s.kind == SlotKind::Fortress);
                if let Some(pf) = prev_fort {
                    // Build grid: open locks up to prev_lock's section, close the rest + candidate
                    let mut g = base_grid.clone();
                    for l in &locks {
                        if l.fort_section < prev_lock.fort_section {
                            g.set(l.pos.0, l.pos.1, l.replace_tile);
                        } else {
                            g.set(l.pos.0, l.pos.1, l.gap_tile);
                        }
                    }
                    // Also place the candidate lock
                    g.set(cand_pos.0, cand_pos.1, gap);
                    let w = walk_map(&g, pipe_pairs, start_pos, world_idx);
                    !w.nodes.contains(&pf.pos)
                } else {
                    false
                }
            });
            if blocks_earlier {
                continue;
            }

            // Check if target is reachable with this lock closed (used for
            // secret exit safety).
            let target_reachable = target_pos
                .map(|tp| walk.nodes.contains(&tp))
                .unwrap_or(true);

            // A "safe" lock blocks nothing important: all fortresses and
            // the target remain reachable. Safe for 1-F secret exit since
            // leaving it closed can never cause a softlock.
            let safe = target_reachable && slots.iter().all(|s| {
                s.kind != SlotKind::Fortress || walk.nodes.contains(&s.pos)
            });

            // Score by gated node count: how many nodes become unreachable
            // when this lock is closed? Prefers chokepoints that gate large
            // portions of the map over locks adjacent to the airship (which
            // only gate ~1 node).
            let gated = open_node_count - walk.nodes.len() as i32;

            let mut score: i32 = gated;

            // Bonus: blocks a later fortress (strong progression signal)
            let blocks_later_fort = slots.iter().any(|s| {
                s.kind == SlotKind::Fortress
                    && s.section > section_idx
                    && !walk.nodes.contains(&s.pos)
            });
            if blocks_later_fort {
                score += knobs.blocks_later_fort_bonus;
            }

            // Bonus: blocks the target (airship/bowser) — only credited to
            // the first such lock in the world; subsequent target-blockers
            // would just pile up next to the airship.
            if !target_reachable && !target_already_locked {
                score += knobs.blocks_target_bonus;
            }

            // Spread penalty: discourage placing this lock close to any
            // already-placed lock in the world. Falls off linearly with
            // Manhattan distance, zero past `spread_radius` tiles.
            if let Some(min_dist) = locks
                .iter()
                .map(|l| cand_pos.0.abs_diff(l.pos.0) + cand_pos.1.abs_diff(l.pos.1))
                .min()
            {
                score -= (knobs.spread_radius - min_dist as i32).max(0)
                    * knobs.spread_penalty_per_tile;
            }

            // Slight preference for bridge tiles — water gaps look better
            // than locks on regular path tiles.
            if tile == 0xB3 {
                score += knobs.bridge_bonus;
            }

            // W8-specific: bias harder toward the screen-3 showcase bridges so
            // they're gated out more often than raw chokepoint value alone
            // would pick them.
            if world_idx == 7 && W8_BRIDGE_LOCK_POSITIONS.contains(&cand_pos) {
                score += knobs.w8_bridge_bonus;
            }

            // (A safe lock never blocks the target, so its blocks_target is
            // false.)
            scored.push(ScoredLock {
                pos: cand_pos,
                gap_tile: gap,
                replace_tile: tile,
                score,
                safe,
                blocks_target: !target_reachable,
            });
        }

        // Sort desc by score. Stable — keeps the shuffled candidate order
        // among ties, matching the old first-best-wins behavior.
        scored.sort_by_key(|c| std::cmp::Reverse(c.score));
        let mut ordered = scored;

        // Gate guarantee: if this section owes the goal a gate, restrict to
        // candidates that actually sever it; carry the duty forward when
        // this section's geometry can't.
        if gate_pending == Some(section_idx) {
            let gating: Vec<ScoredLock> =
                ordered.iter().copied().filter(|c| c.blocks_target).collect();
            if gating.is_empty() {
                if section_idx + 1 < fort_count {
                    gate_pending = Some(section_idx + 1);
                }
                // else: no section could sever the goal — fall through; the
                // blocks_target_bonus soft backstop has done what it can.
            } else {
                ordered = gating;
                gate_pending = None;
            }
        }

        // Prefer safe when forced (retry path) or when the best candidate is
        // weak anyway — don't spend an impactful-lock slot on a weak
        // chokepoint. Safe candidates float to the front, order kept within
        // each half. (Gate candidates are never safe, so an active gate
        // filter is unaffected.)
        let best_score = ordered.first().map(|c| c.score).unwrap_or(0);
        if force_safe || best_score < knobs.weak_lock_threshold {
            let (safe_first, rest): (Vec<ScoredLock>, Vec<ScoredLock>) =
                ordered.into_iter().partition(|c| c.safe);
            ordered = safe_first;
            ordered.extend(rest);
        }

        // Choice guard: demote a lock that would shrink the in-band route
        // count. Bounded by `choice_guard_tries`; falls back to the top
        // candidate when every measured one degrades (a gate lock may
        // legitimately cost a zero-fort route).
        let chosen = if ordered.len() <= 1 {
            ordered.first().copied()
        } else {
            let baseline = measure_routes(locks.clone());
            let mut pick: Option<ScoredLock> = None;
            for cand in ordered.iter().take(knobs.choice_guard_tries) {
                let mut trial = locks.clone();
                trial.push(LockAssignment {
                    pos: cand.pos,
                    gap_tile: cand.gap_tile,
                    replace_tile: cand.replace_tile,
                    fort_section: section_idx,
                    secret_exit_safe: cand.safe,
                    blocks_target: cand.blocks_target,
                });
                if measure_routes(trial) >= baseline {
                    pick = Some(*cand);
                    break;
                }
            }
            pick.or_else(|| ordered.first().copied())
        };

        if let Some(c) = chosen {
            locked_tiles.insert(c.pos);
            locks.push(LockAssignment {
                pos: c.pos,
                gap_tile: c.gap_tile,
                replace_tile: c.replace_tile,
                fort_section: section_idx,
                secret_exit_safe: c.safe,
                blocks_target: c.blocks_target,
            });
        }
    }

    locks
}

/// Stamp build results onto the ROM tile grids for visual inspection.
///
/// Writes generic tiles for each slot type so the overworld maps can be
/// viewed in an emulator. The game will crash if you enter any level.
#[allow(dead_code)]
pub(super) fn debug_stamp_rom(rom: &mut crate::rom::Rom, result: &BuildResult) {
    for built in &result.worlds {
        let wi = built.world_idx;

        // First write the cleared grid (with pipes already placed)
        for r in 0..built.grid.rows() {
            for c in 0..built.grid.cols {
                let offset = rom_data::map_tile_offset(wi, r, c);
                rom.data[offset] = built.grid.get(r, c);
            }
        }

        // Stamp slot assignments
        let mut level_num: u8 = 1;
        for slot in &built.slots {
            let tile = match slot.kind {
                SlotKind::Level => {
                    // Use numbered map tiles ($03-$0D = levels 1-11, then wrap)
                    let t = 0x02 + level_num.min(13);
                    level_num = level_num.wrapping_add(1);
                    t
                }
                SlotKind::Fortress => TILE_FORTRESS,
                SlotKind::Pipe => TILE_PIPE,
                SlotKind::BonusGame => TILE_BONUS_GAME,
                SlotKind::ToadHouse => TILE_TOAD_HOUSE,
                SlotKind::HammerBro => continue, // keep existing blank path tile
            };
            let offset = rom_data::map_tile_offset(wi, slot.pos.0, slot.pos.1);
            rom.data[offset] = tile;
        }

        // Stamp locks
        for lock in &built.locks {
            let offset = rom_data::map_tile_offset(wi, lock.pos.0, lock.pos.1);
            rom.data[offset] = lock.gap_tile;
        }
    }
}
