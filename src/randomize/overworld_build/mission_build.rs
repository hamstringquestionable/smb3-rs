//! Mission-first builder — iteration 1, slices 1–2.
//!
//! Slice 1: translate a mission `Embedding` (abstract node ids from the mission
//! engine) into the builder's own fortress slots and lock assignments.
//!
//! Slice 2: `mission_build`, a drop-in replacement for [`super::build`] that
//! emits a complete `BuildResult`. Per world: place connectivity pipes (reused
//! helper), embed the world's mission (a full fort chain — the progression is
//! decided first and realized by construction), spread levels evenly over the
//! remaining blanks, then fill with hammer-bro slots and reuse the existing
//! spare-pipe / toad-house / spade / HB-sprite passes unchanged. The writer
//! consumes the result exactly as it consumes the old builder's.

use std::collections::HashSet;

use rand::Rng;
use rand::seq::SliceRandom;

use super::capacity::{
    SPADE_BUDGET, assign_hb_sprites, distribute_levels, prepare_capacities, promote_hb_slots,
    redistribute_fortresses,
};
use super::pipes::{
    FIXED_PIPE_ENDPOINTS, PIPE_EXCLUDED_POSITIONS, VANILLA_PIPE_PAIRS, place_pipes,
    place_spare_pipes,
};
use super::plan::{self, Archetype, LockRole, WorldPlan};
use super::scoring::{LEVEL_SPREAD_EXPONENT, VANILLA_LEVEL_COUNT, is_row78_conflict};
use super::sections::{completable_positions, find_blank_slots};
use super::types::{
    BuildFlags, BuildResult, BuiltWorld, CapacityPrep, LockAssignment, OverworldData,
    SlotAssignment, SlotKind, WorldSlotCounts,
};
use crate::randomize::map_walker::walk_map;
use crate::randomize::mission::{GridMap, MapView, Mission, Role, embed};
use crate::randomize::node_catalog::NodeKind;
use crate::randomize::overworld_helpers::{find_target, gap_tile_for};
use crate::randomize::rom_data::{self, Grid, Pos, TeleportEdge};
use crate::rom::Rom;

/// Attempts at re-embedding when an embedding lands two mission pieces on a
/// row-7/8 shared-completion-bit pair. Each failed attempt excludes the
/// conflicting positions before retrying, so the loop always makes progress.
const EMBED_RETRIES: usize = 12;

/// Execute the mission-first build for all 8 worlds. Same contract as
/// [`super::build`]: the writer consumes the returned `BuildResult` unchanged.
/// Selected by the experimental `mission_overworld` option.
pub(crate) fn mission_build<R: Rng>(
    rom: &Rom,
    data: &OverworldData,
    rng: &mut R,
    flags: BuildFlags,
) -> BuildResult {
    let BuildFlags {
        shuffle_toad_houses,
        eights_are_wild,
        shuffle_hammer_bros,
    } = flags;
    let pickup = data.pickup;
    let catalog = data.catalog;

    // Step 0-2 are shared with the old builder verbatim: fort counts, grids +
    // capacities, level distribution.
    let fort_counts = redistribute_fortresses(rng);
    let CapacityPrep {
        patched_grids,
        fixed_positions,
        capacities,
    } = prepare_capacities(
        rom, catalog, pickup, &fort_counts,
        eights_are_wild, shuffle_toad_houses, shuffle_hammer_bros,
    );
    let level_counts = distribute_levels(&capacities, VANILLA_LEVEL_COUNT, LEVEL_SPREAD_EXPONENT, rng);

    // Per-world shapes come from the SAME family-weighted sampler as the old
    // builder (`WorldPlan::sample`: Chain / SingleGate / Fork), so the two
    // builders draw identical shape distributions and remain comparable. The
    // plan's LockRole vocabulary translates 1:1 into a `Mission`
    // (`mission_from_plan`); the mission engine then realizes it by
    // construction instead of hoping scoring produces it.
    //
    // Seed invariant: at least one Safe role somewhere (a Safe lock strands
    // nothing, so it is secret-exit-safe BY CONSTRUCTION — no post-build
    // force_safe retry needed). Multi-fort SingleGate/Fork plans always carry
    // Safes; `ensure_seed_safe_role` covers the all-Chain seed.
    let mut plans: Vec<WorldPlan> = (0..8)
        .map(|wi| WorldPlan::sample(fort_counts[wi], wi, rng))
        .collect();
    plan::ensure_seed_safe_role(&mut plans, rng);

    let mut worlds = Vec::with_capacity(8);
    for wi in 0..8 {
        // Same pointer-table cap as the old builder (see `build`).
        let ptr_slots = pickup.worlds[wi].pool_indices.len();
        let pipe_endpoints = VANILLA_PIPE_PAIRS[wi] * 2;
        let max_non_pipe_slots = ptr_slots.saturating_sub(pipe_endpoints);

        let counts = WorldSlotCounts {
            fort_count: fort_counts[wi],
            level_count: level_counts[wi],
            pipe_pair_count: VANILLA_PIPE_PAIRS[wi],
            max_non_pipe_slots,
            force_safe: false,
        };
        worlds.push(mission_build_world(
            wi,
            rom,
            patched_grids[wi].clone(),
            &fixed_positions[wi],
            &counts,
            &plans[wi],
            shuffle_hammer_bros,
            rng,
        ));
    }

    // Filler promotion + HB sprites: identical to the old builder.
    promote_hb_slots(
        rom, &mut worlds, data, rng,
        |k| matches!(k, NodeKind::ToadHouse), SlotKind::ToadHouse, None,
    );
    promote_hb_slots(
        rom, &mut worlds, data, rng,
        |k| matches!(k, NodeKind::BonusGame), SlotKind::BonusGame, Some(SPADE_BUDGET),
    );
    if shuffle_hammer_bros {
        assign_hb_sprites(rom, data.pickup, &mut worlds, rng);
    }

    BuildResult { worlds, fort_counts }
}

/// Translate a `WorldPlan`'s lock roles into a `Mission` — the two vocabularies
/// are deliberately 1:1 (`LockRole` grew out of the same lock-and-key model).
fn mission_from_plan(plan: &WorldPlan) -> Mission {
    let roles = plan
        .roles
        .iter()
        .map(|r| match r {
            LockRole::ChainLink { targets } => Role::ChainLink { targets: targets.clone() },
            LockRole::GoalGate => Role::GoalGate,
            LockRole::Safe => Role::Safe,
        })
        .collect();
    Mission { roles }
}

/// Build one world mission-first. Mirrors `build_world`'s step order (pipes →
/// forts+locks → levels → HB filler → slot cap → spare pipes) but replaces the
/// scored fort/lock/level placement with the mission embed + even spread.
// Reason: same call shape as `build_world` — each arg is a distinct build
// input; no subset forms a concept worth a struct.
#[allow(clippy::too_many_arguments)]
fn mission_build_world<R: Rng>(
    world_idx: usize,
    rom: &Rom,
    mut grid: Grid,
    fixed_positions: &HashSet<Pos>,
    counts: &WorldSlotCounts,
    plan: &WorldPlan,
    shuffle_hammer_bros: bool,
    rng: &mut R,
) -> BuiltWorld {
    let start_pos = rom_data::find_start(&grid);
    let target_pos = find_target(&grid, world_idx);
    let blank_positions = find_blank_slots(&grid, fixed_positions);

    // Step 1: connectivity pipes — reused helper, default knobs.
    let fixed_pipe_eps: Vec<Pos> = FIXED_PIPE_ENDPOINTS
        .iter()
        .filter(|(wi, _)| *wi == world_idx)
        .map(|(_, pos)| *pos)
        .collect();
    let pipe_excluded: HashSet<Pos> = PIPE_EXCLUDED_POSITIONS
        .iter()
        .filter(|(wi, _)| *wi == world_idx)
        .map(|(_, pos)| *pos)
        .collect();
    let pipe_blanks: Vec<Pos> = blank_positions
        .iter()
        .copied()
        .filter(|p| !pipe_excluded.contains(p))
        .collect();
    let mut pipe_pairs = place_pipes(
        &mut grid,
        &pipe_blanks,
        start_pos,
        target_pos,
        counts.pipe_pair_count,
        &fixed_pipe_eps,
        &plan.pipe,
        world_idx,
        rng,
    );
    let pipe_positions: HashSet<Pos> = pipe_pairs.iter().flat_map(|&(a, b)| [a, b]).collect();

    // Step 2: embed the mission — forts + locks realized by construction.
    // `realized_plan` is the plan that actually embedded (== `plan` unless the
    // simplification ladder had to step in), kept truthful for diagnostics.
    let (fort_slots, locks, realized_plan) =
        embed_mission(&grid, &pipe_pairs, world_idx, plan, fixed_positions, rng);
    let section_count = fort_slots.len();
    let fort_positions: HashSet<Pos> = fort_slots.iter().map(|s| s.pos).collect();

    // Pipe endpoints get slots first (in pipe-pair order — deterministic),
    // then the forts.
    let mut slots: Vec<SlotAssignment> = pipe_pairs
        .iter()
        .flat_map(|&(a, b)| [a, b])
        .map(|pos| SlotAssignment {
            pos,
            kind: SlotKind::Pipe,
            section: 0,
            is_hand_trap: false,
            is_troll_pipe: false,
        })
        .collect();
    slots.extend(fort_slots);

    // Step 3: levels, spread as evenly as possible (the charter's unifying
    // placement rule): repeatedly take the candidate farthest (min Manhattan
    // distance) from all content placed so far. The up-front shuffle makes
    // maximin ties land randomly per seed.
    let reachable = walk_map(&grid, &pipe_pairs, start_pos, world_idx).nodes;
    let mut completable = completable_positions(&grid, &slots);
    for l in &locks {
        completable.insert(l.pos);
    }

    let mut candidates: Vec<Pos> = blank_positions
        .iter()
        .copied()
        .filter(|p| {
            reachable.contains(p) && !pipe_positions.contains(p) && !fort_positions.contains(p)
        })
        .collect();
    candidates.shuffle(rng);

    let mut anchors: Vec<Pos> = fort_positions
        .iter()
        .chain(pipe_positions.iter())
        .copied()
        .collect();
    let mut level_positions: Vec<Pos> = Vec::new();
    for _ in 0..counts.level_count {
        let pick = candidates
            .iter()
            .copied()
            .filter(|&p| !is_row78_conflict(p, &completable))
            .max_by_key(|&p| {
                anchors
                    .iter()
                    .map(|&(ar, ac)| ar.abs_diff(p.0) + ac.abs_diff(p.1))
                    .min()
                    .unwrap_or(usize::MAX)
            });
        let Some(pos) = pick else { break };
        candidates.retain(|&p| p != pos);
        level_positions.push(pos);
        anchors.push(pos);
        completable.insert(pos);
    }
    for &pos in &level_positions {
        slots.push(SlotAssignment {
            pos,
            kind: SlotKind::Level,
            section: 0,
            is_hand_trap: false,
            is_troll_pipe: false,
        });
    }

    // Mandatory HammerBro slots under the vanilla sprite positions (same as
    // `build_world` — the sprite starts there and needs a pointer entry).
    let hb_sprite_pos_list: Vec<Pos> = if shuffle_hammer_bros {
        Vec::new()
    } else {
        let existing: HashSet<Pos> = slots.iter().map(|s| s.pos).collect();
        rom_data::read_hb_sprite_positions(rom, world_idx)
            .into_iter()
            .filter(|pos| !existing.contains(pos))
            .collect()
    };
    let hb_sprite_positions: HashSet<Pos> = hb_sprite_pos_list.iter().copied().collect();
    for pos in &hb_sprite_pos_list {
        slots.push(SlotAssignment {
            pos: *pos,
            kind: SlotKind::HammerBro,
            section: 0,
            is_hand_trap: false,
            is_troll_pipe: false,
        });
    }

    // Remaining reachable blanks become HammerBro filler slots.
    for &pos in &candidates {
        if hb_sprite_positions.contains(&pos) {
            continue;
        }
        slots.push(SlotAssignment {
            pos,
            kind: SlotKind::HammerBro,
            section: 0,
            is_hand_trap: false,
            is_troll_pipe: false,
        });
    }

    // Cap total slots to what the pointer table can hold — verbatim from
    // `build_world`: only regular HB slots are dropped, never sprite HBs.
    if slots.len() > counts.max_non_pipe_slots {
        let mut kept: Vec<SlotAssignment> = Vec::with_capacity(counts.max_non_pipe_slots);
        let mut hb_slots: Vec<SlotAssignment> = Vec::new();
        for s in slots {
            if s.kind != SlotKind::HammerBro || hb_sprite_positions.contains(&s.pos) {
                kept.push(s);
            } else {
                hb_slots.push(s);
            }
        }
        let hb_budget = counts.max_non_pipe_slots.saturating_sub(kept.len());
        kept.extend(hb_slots.into_iter().take(hb_budget));
        slots = kept;
    }

    // Step 4: spare pipes — reused helper, runs with locks already placed so
    // its lock-aware fort-skip / goal-open rejection applies as-is.
    let spare_needed = counts.pipe_pair_count.saturating_sub(pipe_pairs.len());
    place_spare_pipes(
        &mut grid,
        &mut slots,
        &mut pipe_pairs,
        spare_needed,
        &hb_sprite_positions,
        &locks,
        &plan.pipe,
        start_pos,
        target_pos,
        world_idx,
        rng,
    );

    BuiltWorld {
        world_idx,
        grid,
        slots,
        locks,
        section_count,
        pipe_pairs,
        hb_sprites: Vec::new(),
        plan: realized_plan,
    }
}

/// Embed the plan's mission under two HARD rules: every budgeted fort is
/// placed, and every fort gets a real lock. Neither ever degrades — if the
/// map can't host the sampled shape, only the ROLES simplify: SingleGate,
/// then Chain, then all-Safe (every lock gates nothing important — the same
/// floor as the old builder's force_safe shape). Returns the forts, locks,
/// and the plan that actually embedded, so `BuiltWorld.plan` stays truthful.
///
/// A map that can't even host all-Safe (n free-standing forts + n harmless
/// locks) has no valid placement under the hard rules at all; that's a bug
/// in the geometry pipeline, not a shape problem, so it panics loudly
/// instead of silently shipping a world with missing content.
fn embed_mission<R: Rng>(
    grid: &Grid,
    pipes: &[TeleportEdge],
    world_idx: usize,
    plan: &WorldPlan,
    excluded_forts: &HashSet<Pos>,
    rng: &mut R,
) -> (Vec<SlotAssignment>, Vec<LockAssignment>, WorldPlan) {
    let n = plan.roles.len();
    // The sampled plan leads; fallbacks re-derive fresh role lists (the
    // sampled one may carry an ensure_seed_safe_role demotion, so a same-shape
    // fallback is not always redundant).
    let mut ladder: Vec<WorldPlan> = vec![plan.clone()];
    if n >= 2 && plan.archetype != Archetype::SingleGate {
        ladder.push(WorldPlan::from_archetype(Archetype::SingleGate, n));
    }
    ladder.push(WorldPlan::from_archetype(Archetype::Chain, n));
    ladder.push(WorldPlan::all_safe(n));

    let mut tried: Vec<Vec<Role>> = Vec::new();
    for cand in ladder {
        let mission = mission_from_plan(&cand);
        if tried.contains(&mission.roles) {
            continue; // identical role list already failed (e.g. Chain(1) == SingleGate(1))
        }
        tried.push(mission.roles.clone());
        if let Some((forts, locks)) =
            mission_forts_and_locks(grid, pipes, world_idx, &mission, excluded_forts, rng)
        {
            return (forts, locks, cand);
        }
    }
    panic!(
        "W{}: no {n}-fort mission embeds on this grid — even all-Safe failed \
         (fort/lock hard rules are unsatisfiable; geometry pipeline bug)",
        world_idx + 1
    );
}

/// Place this world's fortresses and locks mission-first: embed `mission` on
/// the cleared + piped grid and translate the result into builder slots +
/// locks. `None` if the map can't host the mission (rare — embed is ~100% on
/// cleared grids).
///
/// Two constraints the mission engine can't see are enforced here:
/// - no fort on an excluded (fixed sprite) position, and no fort/lock whose
///   row-7/8 partner column already holds a completion-relevant map tile
///   (candidates are filtered up front);
/// - no two mission pieces on a row-7/8 pair of the same column (checked after
///   embedding; on a hit the offending positions are excluded and the embed
///   reruns — see `EMBED_RETRIES`).
fn mission_forts_and_locks<R: Rng>(
    grid: &Grid,
    pipes: &[TeleportEdge],
    world_idx: usize,
    mission: &Mission,
    excluded_forts: &HashSet<Pos>,
    rng: &mut R,
) -> Option<(Vec<SlotAssignment>, Vec<LockAssignment>)> {
    let fort_count = mission.fort_count();
    if fort_count == 0 {
        return Some((Vec::new(), Vec::new()));
    }

    let mut gm = GridMap::new(grid.clone(), pipes.to_vec(), world_idx)?;
    let static_completable = completable_positions(grid, &[]);
    gm.exclude(
        |p| excluded_forts.contains(&p) || is_row78_conflict(p, &static_completable),
        |p| is_row78_conflict(p, &static_completable),
    );

    let cols = grid.cols;
    let decode = |id: usize| (id / cols, id % cols);

    for _ in 0..EMBED_RETRIES {
        gm.shuffle_candidates(rng);
        // Embed is a complete search over the current candidates: `None` here
        // means the mission genuinely doesn't fit, and retrying can't help.
        let placed = embed(mission, &gm)?;

        // Mission pieces are all completion-relevant, so no two of them may
        // share a row-7/8 column pair (static tiles were pre-filtered above).
        let pieces: HashSet<Pos> = placed
            .fort_pos
            .iter()
            .chain(placed.lock_pos.iter())
            .map(|&id| decode(id))
            .collect();
        let conflicted: HashSet<Pos> = pieces
            .iter()
            .filter(|&&(r, c)| {
                (r == 7 && pieces.contains(&(8, c))) || (r == 8 && pieces.contains(&(7, c)))
            })
            .copied()
            .collect();
        if !conflicted.is_empty() {
            gm.exclude(|p| conflicted.contains(&p), |p| conflicted.contains(&p));
            continue;
        }

        let mut forts = Vec::with_capacity(fort_count);
        let mut locks = Vec::with_capacity(fort_count);
        for fort in 0..fort_count {
            forts.push(SlotAssignment {
                pos: decode(placed.fort_pos[fort]),
                kind: SlotKind::Fortress,
                section: fort,
                is_hand_trap: false,
                is_troll_pipe: false,
            });

            let lock_id = placed.lock_pos[fort];
            let lock_pos = decode(lock_id);
            let original = grid.get(lock_pos.0, lock_pos.1);
            // With this lock alone closed, is the goal cut off? That's a
            // target-blocker; otherwise the secret exit stays safe.
            let blocks_target = gm.strands(lock_id, gm.goal());
            locks.push(LockAssignment {
                pos: lock_pos,
                gap_tile: gap_tile_for(original),
                replace_tile: original,
                fort_section: fort,
                secret_exit_safe: !blocks_target,
                blocks_target,
            });
        }
        return Some((forts, locks));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::randomize::node_catalog::NodeCatalog;
    use crate::randomize::overworld_helpers::LOCKABLE_TILES;
    use crate::randomize::overworld_pickup::{PickupFlags, pick_up};
    use crate::rom::Rom;
    use rand::SeedableRng;
    use rand_chacha::ChaCha8Rng;
    use std::collections::HashSet;

    fn load_rom() -> Option<Rom> {
        let data = std::fs::read("roms/Super Mario Bros. 3 (USA) (Rev 1).nes").ok()?;
        Rom::from_bytes(&data).ok()
    }

    /// Slice-2 check: `mission_build` emits a complete, well-formed
    /// `BuildResult` on the real ROM — full-size mission embeds, well-formed
    /// locks, the seed-wide secret-exit-safe invariant, the full level budget,
    /// and no row-7/8 completion-bit conflicts among the pieces it places.
    #[test]
    fn mission_build_emits_full_build_result() {
        let Some(rom) = load_rom() else {
            return; // no ROM — skip
        };
        let catalog = NodeCatalog::build(&rom, false);
        let pickup = pick_up(
            &rom,
            &catalog,
            PickupFlags {
                shuffle_spade_games: false,
                shuffle_toad_houses: true,
                shuffle_hammer_bros: false,
            },
        );
        let data = OverworldData { pickup: &pickup, catalog: &catalog };

        for seed in 0..10u64 {
            let mut rng = ChaCha8Rng::seed_from_u64(seed);
            let result = mission_build(
                &rom,
                &data,
                &mut rng,
                BuildFlags { shuffle_toad_houses: true, ..Default::default() },
            );

            assert_eq!(result.worlds.len(), 8);
            let mut total_levels = 0usize;
            let mut any_safe = false;

            for built in &result.worlds {
                let wi = built.world_idx;
                let fc = result.fort_counts[wi];

                // Mission realization: every world embeds at full size.
                let fort_slots: Vec<_> = built
                    .slots
                    .iter()
                    .filter(|s| s.kind == SlotKind::Fortress)
                    .collect();
                assert_eq!(
                    fort_slots.len(), fc,
                    "W{} seed {seed}: mission embedded {} of {fc} forts",
                    wi + 1,
                    fort_slots.len()
                );
                assert_eq!(built.section_count, fc, "W{} section_count", wi + 1);
                let sections: HashSet<usize> = fort_slots.iter().map(|s| s.section).collect();
                assert_eq!(sections, (0..fc).collect(), "W{} fort sections", wi + 1);

                // Slot sanity: all positions distinct.
                let all_pos: HashSet<Pos> = built.slots.iter().map(|s| s.pos).collect();
                assert_eq!(all_pos.len(), built.slots.len(), "W{} duplicate slot", wi + 1);

                // Locks: one per fort, on lockable tiles, off the slots.
                assert_eq!(built.locks.len(), fc, "W{} lock count", wi + 1);
                let lock_positions: HashSet<Pos> = built.locks.iter().map(|l| l.pos).collect();
                assert_eq!(lock_positions.len(), fc, "W{} distinct locks", wi + 1);
                for l in &built.locks {
                    assert!(
                        LOCKABLE_TILES.contains(&l.replace_tile),
                        "W{} lock on non-lockable tile {:#04x}",
                        wi + 1,
                        l.replace_tile
                    );
                    assert!(l.fort_section < fc, "W{} lock section", wi + 1);
                    assert!(!all_pos.contains(&l.pos), "W{} lock on a slot tile", wi + 1);
                }
                any_safe |= built.locks.iter().any(|l| l.secret_exit_safe);

                // Row-7/8 completion-bit audit over the pieces this builder
                // places itself (forts, levels, locks). Promoted toad houses /
                // spades are guarded inside promote_hb_slots.
                let mut completable = completable_positions(&built.grid, &built.slots);
                for l in &built.locks {
                    completable.insert(l.pos);
                }
                let ours = built
                    .slots
                    .iter()
                    .filter(|s| matches!(s.kind, SlotKind::Level | SlotKind::Fortress))
                    .map(|s| s.pos)
                    .chain(built.locks.iter().map(|l| l.pos));
                for (r, c) in ours {
                    let partner = match r {
                        7 => Some((8, c)),
                        8 => Some((7, c)),
                        _ => None,
                    };
                    if let Some(p) = partner {
                        assert!(
                            !completable.contains(&p),
                            "W{} seed {seed}: row-7/8 conflict at ({r},{c})",
                            wi + 1
                        );
                    }
                }

                total_levels += built
                    .slots
                    .iter()
                    .filter(|s| s.kind == SlotKind::Level)
                    .count();

                // Pipes stay within the fixed per-world budget.
                assert!(
                    built.pipe_pairs.len() <= VANILLA_PIPE_PAIRS[wi],
                    "W{} pipe budget",
                    wi + 1
                );
            }

            assert_eq!(total_levels, VANILLA_LEVEL_COUNT, "seed {seed}: level total");
            assert!(any_safe, "seed {seed}: no secret-exit-safe lock in the seed");
        }
    }

    /// Shape-variety scoreboard: can non-chain shapes embed on real grids?
    /// For every world of every seed's mission build, try embedding EACH shape
    /// at the world's full fort count on the world's actual piped grid, and
    /// report per-shape realization; also reports the archetype census the
    /// builds actually shipped (truthful `BuiltWorld.plan`).
    ///
    /// NOTE: built grids include spare pipes (production embeds BEFORE those
    /// are placed), so probe rates here are if anything slightly pessimistic —
    /// extra teleports only make stranding harder.
    ///
    /// Ignored by default: probing every shape on every world (including
    /// exhaustively proving the rare impossible fork) takes ~70s in release.
    /// The fast invariants test above already guards shipped builds.
    /// Run with: cargo test --release --lib mission_shape_scoreboard -- --ignored --nocapture
    #[test]
    #[ignore = "on-demand scoreboard diagnostic (~70s release)"]
    fn mission_shape_scoreboard() {
        use std::collections::BTreeMap;

        let Some(rom) = load_rom() else {
            return; // no ROM — skip
        };
        let catalog = NodeCatalog::build(&rom, false);
        let pickup = pick_up(
            &rom,
            &catalog,
            PickupFlags {
                shuffle_spade_games: false,
                shuffle_toad_houses: true,
                shuffle_hammer_bros: false,
            },
        );
        let data = OverworldData { pickup: &pickup, catalog: &catalog };

        const SEEDS: u64 = 30;
        let label = |a: Archetype| match a {
            Archetype::Chain => "chain".to_string(),
            Archetype::SingleGate => "single-gate".to_string(),
            Archetype::Fork { k } => format!("fork-{k}"),
        };

        // shape label -> (attempts, full-size embeds)
        let mut probe: BTreeMap<String, (u32, u32)> = BTreeMap::new();
        // realized archetype label -> world count
        let mut shipped: BTreeMap<String, u32> = BTreeMap::new();
        let mut full_size = 0u32;
        let mut worlds_total = 0u32;

        for seed in 0..SEEDS {
            let mut rng = ChaCha8Rng::seed_from_u64(seed);
            let result = mission_build(
                &rom,
                &data,
                &mut rng,
                BuildFlags { shuffle_toad_houses: true, ..Default::default() },
            );
            let CapacityPrep { fixed_positions, .. } =
                prepare_capacities(&rom, &catalog, &pickup, &result.fort_counts, false, true, false);

            for built in &result.worlds {
                let wi = built.world_idx;
                let n = result.fort_counts[wi];
                worlds_total += 1;
                if built.section_count == n {
                    full_size += 1;
                }
                *shipped.entry(label(built.plan.archetype)).or_default() += 1;

                let mut shapes = vec![Archetype::Chain];
                if n >= 2 {
                    shapes.push(Archetype::SingleGate);
                    shapes.push(Archetype::Fork { k: 2 });
                }
                if n >= 3 {
                    shapes.push(Archetype::Fork { k: 3 });
                }
                for arch in shapes {
                    let plan = WorldPlan::from_archetype(arch, n);
                    let mission = mission_from_plan(&plan);
                    let ok = mission_forts_and_locks(
                        &built.grid,
                        &built.pipe_pairs,
                        wi,
                        &mission,
                        &fixed_positions[wi],
                        &mut rng,
                    )
                    .is_some();
                    let e = probe.entry(label(arch)).or_default();
                    e.0 += 1;
                    if ok {
                        e.1 += 1;
                    }
                }
            }
        }

        eprintln!("\nShape embed rates on real piped grids ({SEEDS} seeds x 8 worlds):\n");
        eprintln!("  {:<12} {:>9} {:>9} {:>7}", "shape", "attempts", "embeds", "rate");
        for (l, (att, ok)) in &probe {
            eprintln!(
                "  {:<12} {:>9} {:>9} {:>6.1}%",
                l, att, ok,
                *ok as f64 / (*att).max(1) as f64 * 100.0
            );
        }
        eprintln!("\nShipped archetype census (realized plans):");
        for (l, c) in &shipped {
            eprintln!("  {:<12} {:>4} ({:>4.1}%)", l, c, *c as f64 / worlds_total as f64 * 100.0);
        }
        eprintln!(
            "\n  full-size embeds in shipped builds: {full_size}/{worlds_total} ({:.1}%)",
            full_size as f64 / worlds_total as f64 * 100.0
        );

        // Regression floor — every shape must embed on the vast majority of
        // real grids, and shipped builds must essentially never shrink.
        for (l, (att, ok)) in &probe {
            let rate = *ok as f64 / (*att).max(1) as f64;
            assert!(rate >= 0.9, "shape '{l}' embed rate {:.1}% below 90%", rate * 100.0);
        }
        assert_eq!(full_size, worlds_total, "shipped builds shrank a mission");
    }
}
