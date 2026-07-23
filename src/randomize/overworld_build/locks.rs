//! Fortress lock / water-gap placement.

use super::*;

use super::types::{BuildResult, LockAssignment, SlotAssignment, SlotKind, stamp_slots};

/// The five always-on W8 screen-3 bridges (see `qol::overworld_map`'s
/// `W8_BRIDGE_EDITS`). Locking one gaps it out as water until its fortress is
/// beaten — a deliberate showcase, so the lock scorer nudges toward them.
const W8_BRIDGE_LOCK_POSITIONS: &[(usize, usize)] = &[(5, 51), (5, 53), (5, 55), (5, 57), (5, 59)];

/// Score bonus for locking a W8 showcase bridge. Sized to noticeably raise how
/// often a bridge is chosen without overriding the `blocks_later_fort` (+100)
/// progression signal or the hard reachability rules. Tuned via a 200k-seed
/// sweep: +8 puts ≥1 bridge out in ~99.6% of seeds, two out in ~30%, and all
/// four (the ceiling — W8 has 4 forts = 4 locks) out in ~0.08% (a rare treat).
const W8_BRIDGE_LOCK_BONUS: i32 = 8;

/// The role a fortress's lock plays in the world's progression archetype.
/// Decided per-world in [`sample_lock_plan`] and consumed by [`place_locks`],
/// which places each lock to satisfy its role using the shared positional
/// scorer. See the `fort_topology_archetypes` design notes.
#[derive(Clone, Debug)]
pub(super) enum LockRole {
    /// Lock must gate (make unreachable) every fort section in `targets` — a
    /// chain link (gate the next fort) or the prefix→fork entrance (gate the
    /// whole terminal group at once).
    ChainLink { targets: Vec<usize> },
    /// Lock must gate the airship/Bowser target while stranding no fortress.
    GoalGate,
    /// Lock must gate nothing important (no fort, no target) — the fortress
    /// stays reachable. Decoy and inert forts.
    Safe,
}

/// Weighted per-world archetype sample → per-section lock roles.
///
/// - 1 fort: SingleGate (the lone fort gates the goal).
/// - W8 (dense, 4 forts): 25% Chain / 75% Fork; Fork = chain-2 → 2-way fork.
/// - other multi-fort worlds: 30% Chain / 30% SingleGate / 40% Fork; a Fork
///   samples a terminal width `k` (capped at 3, surplus forts chain in front),
///   so a 3-fort Fork is a 50/50 mix of chain-1→2-fork and a pure 3-way fork.
pub(super) fn sample_lock_plan<R: Rng>(
    fort_count: usize,
    world_idx: usize,
    rng: &mut R,
) -> Vec<LockRole> {
    match fort_count {
        0 => return Vec::new(),
        1 => return vec![LockRole::GoalGate],
        _ => {}
    }

    enum Arch {
        Chain,
        Single,
        Fork,
    }
    let arch = if world_idx == 7 {
        if rng.random_range(..100u32) < 25 { Arch::Chain } else { Arch::Fork }
    } else {
        let r = rng.random_range(..100u32);
        if r < 30 {
            Arch::Chain
        } else if r < 60 {
            Arch::Single
        } else {
            Arch::Fork
        }
    };

    match arch {
        Arch::Chain => (0..fort_count)
            .map(|i| {
                if i + 1 < fort_count {
                    LockRole::ChainLink { targets: vec![i + 1] }
                } else {
                    LockRole::GoalGate
                }
            })
            .collect(),
        Arch::Single => (0..fort_count)
            .map(|i| if i + 1 == fort_count { LockRole::GoalGate } else { LockRole::Safe })
            .collect(),
        Arch::Fork => {
            // W8 and 2-fort worlds: a plain 2-way fork. 3-fort worlds mix
            // 50/50 between chain-1 → 2-fork (k=2) and a pure 3-way fork (k=3).
            let k = if world_idx == 7 || fort_count == 2 {
                2
            } else {
                2 + rng.random_range(..2u32) as usize
            };
            fork_roles(fort_count, k)
        }
    }
}

/// Chain the first `n - k` forts, then a `k`-way fork (one GoalGate + `k-1`
/// Safe decoys). `k` is capped at 3 and at `n`.
fn fork_roles(n: usize, k: usize) -> Vec<LockRole> {
    let k = k.min(n).min(3);
    let prefix = n - k;
    let mut roles = Vec::with_capacity(n);
    for i in 0..prefix {
        let targets = if i + 1 < prefix {
            vec![i + 1]
        } else {
            // Last prefix link opens the whole terminal fork as a unit.
            (prefix..n).collect()
        };
        roles.push(LockRole::ChainLink { targets });
    }
    for i in prefix..n {
        roles.push(if i == n - 1 { LockRole::GoalGate } else { LockRole::Safe });
    }
    roles
}

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
// placement (geometry, slot list, count, safety flag, RNG). No subset
// clusters into a meaningful concept — bundling would be a clippy bandage,
// not a real abstraction.
#[allow(clippy::too_many_arguments)]
pub(super) fn place_locks<R: Rng>(
    grid: &Grid,
    pipe_pairs: &[TeleportEdge],
    start_pos: Option<(usize, usize)>,
    target_pos: Option<(usize, usize)>,
    slots: &[SlotAssignment],
    fort_count: usize,
    roles: &[LockRole],
    force_safe: bool,
    world_idx: usize,
    rng: &mut R,
) -> Vec<LockAssignment> {
    let mut locks: Vec<LockAssignment> = Vec::new();
    let mut locked_tiles: HashSet<(usize, usize)> = HashSet::new();

    // Fort position by section, for evaluating archetype role constraints.
    let fort_by_section: HashMap<usize, (usize, usize)> = slots
        .iter()
        .filter(|s| s.kind == SlotKind::Fortress)
        .map(|s| (s.section, s.pos))
        .collect();

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

        // Prefer safe when forced (retry path) or when the best candidate
        // is weak anyway (score < 5) — don't sacrifice a high-scoring lock.
        // Evaluated after scoring all candidates, see below.
        let mut best: Option<ScoredLock> = None;
        let mut best_safe: Option<ScoredLock> = None;
        // Best candidate that satisfies this section's archetype role, if any.
        let mut best_role: Option<ScoredLock> = None;
        let role = roles.get(section_idx);

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
                score += 100;
            }

            // Bonus: blocks the target (airship/bowser) — only credited to
            // the first such lock in the world; subsequent target-blockers
            // would just pile up next to the airship.
            if !target_reachable && !target_already_locked {
                score += 10;
            }

            // Spread penalty: discourage placing this lock close to any
            // already-placed lock in the world. Falls off linearly with
            // Manhattan distance, zero past 8 tiles.
            if let Some(min_dist) = locks
                .iter()
                .map(|l| cand_pos.0.abs_diff(l.pos.0) + cand_pos.1.abs_diff(l.pos.1))
                .min()
            {
                score -= (8i32 - min_dist as i32).max(0) * 2;
            }

            // Slight preference for bridge tiles — water gaps look better
            // than locks on regular path tiles.
            if tile == 0xB3 {
                score += 1;
            }

            // W8-specific: bias harder toward the screen-3 showcase bridges so
            // they're gated out more often than raw chokepoint value alone
            // would pick them.
            if world_idx == 7 && W8_BRIDGE_LOCK_POSITIONS.contains(&cand_pos) {
                score += W8_BRIDGE_LOCK_BONUS;
            }

            // Track best overall and best safe separately. (A safe lock
            // never blocks the target, so its blocks_target is false.)
            let cand = ScoredLock {
                pos: cand_pos,
                gap_tile: gap,
                replace_tile: tile,
                score,
                safe,
                blocks_target: !target_reachable,
            };
            if best.is_none_or(|b| score > b.score) {
                best = Some(cand);
            }
            if safe && best_safe.is_none_or(|b| score > b.score) {
                best_safe = Some(cand);
            }

            // Does this candidate satisfy the section's archetype role?
            let role_ok = match role {
                Some(LockRole::ChainLink { targets }) => targets.iter().all(|t| {
                    fort_by_section
                        .get(t)
                        .is_some_and(|fp| !walk.nodes.contains(fp))
                }),
                // Gate the target while stranding no fortress.
                Some(LockRole::GoalGate) => {
                    !target_reachable
                        && slots
                            .iter()
                            .all(|s| s.kind != SlotKind::Fortress || walk.nodes.contains(&s.pos))
                }
                Some(LockRole::Safe) => safe,
                None => false,
            };
            if role_ok && best_role.is_none_or(|b| score > b.score) {
                best_role = Some(cand);
            }
        }

        // Prefer safe when forced (retry) or when best score is low —
        // no point picking an impactful lock if there are none.
        let best_score = best.map(|b| b.score).unwrap_or(0);
        let prefer_safe = force_safe || best_score < 5;

        // Archetype role wins when realizable; otherwise fall back to the
        // unconstrained pick (feasibility fallback — this world's geometry
        // couldn't host the sampled shape for this fort).
        let chosen = best_role.or(if prefer_safe { best_safe.or(best) } else { best });

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
