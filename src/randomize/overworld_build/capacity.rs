//! Step 0: per-world capacity budgeting and Hammer-Bro sprite distribution.

use super::*;

use super::types::{BuiltWorld, CapacityPrep, HbSprite, OverworldData, SlotKind};

pub(super) const SPADE_BUDGET: usize = 19;

/// Number of pipe pairs (not endpoints) per world in the vanilla ROM — each
/// world's pipe budget.
pub(crate) const VANILLA_PIPE_PAIRS: [usize; 8] = [
    0,  // W1
    1,  // W2
    3,  // W3
    2,  // W4
    2,  // W5 (includes spiral tower)
    2,  // W6
    8,  // W7
    6,  // W8
];

/// Total action levels distributed across the 8 worlds (the vanilla count).
pub(crate) const VANILLA_LEVEL_COUNT: usize = 62;

/// Exponent compressing per-world capacity shares when distributing levels
/// (0.5 = square root: dense worlds can't hoard).
pub(crate) const LEVEL_SPREAD_EXPONENT: f64 = 0.5;

/// BFS-ordered list of (position, distance) using the canonical `walk_map`.
///
/// This is the single source of truth for map traversal — all BFS-dependent
/// logic must go through here or `walk_map` directly to stay in sync with
/// canoe edges, pipe teleports, etc.
pub(crate) fn bfs_ordered(
    grid: &Grid,
    pipe_pairs: &[TeleportEdge],
    start_pos: Option<(usize, usize)>,
    world_idx: usize,
) -> Vec<((usize, usize), usize)> {
    let result = walk_map(grid, pipe_pairs, start_pos, world_idx);
    let mut ordered: Vec<((usize, usize), usize)> = result
        .distances
        .into_iter()
        .collect();
    // Sort by distance, then by position for determinism (HashMap has no order).
    ordered.sort_by_key(|&((r, c), d)| (d, r, c));
    ordered
}

/// Returns true if placing a completable tile at `pos` would create a
/// row 7/8 completion-bit collision. This is a hard game engine constraint
/// (shared bit $01) that cannot be relaxed.
pub(super) fn is_row78_conflict(
    pos: (usize, usize),
    completable: &HashSet<(usize, usize)>,
) -> bool {
    let (r, c) = pos;
    if r == 7 {
        completable.contains(&(8, c))
    } else if r == 8 {
        completable.contains(&(7, c))
    } else {
        false
    }
}

/// All blank placement slots on a grid, minus fixed positions.
pub(super) fn find_blank_slots(
    grid: &Grid,
    fixed_positions: &HashSet<(usize, usize)>,
) -> Vec<(usize, usize)> {
    let mut blanks = Vec::new();
    for r in 0..grid.rows() {
        for c in 0..grid.cols {
            let pos = (r, c);
            if fixed_positions.contains(&pos) {
                continue;
            }
            if !rom_data::VALID_BLANK_TILES.contains(&grid.get(r, c)) {
                continue;
            }
            blanks.push(pos);
        }
    }
    blanks
}

/// Returns true if a tile would be "caught" by the game's
/// `Map_Reload_with_Completions` routine. Used to seed the completable set
/// and to prevent lock placement at row 7 when row 8 has a level.
///
/// The game checks (in order):
/// 1. Special tiles: $50, $E8, $E6, $BD, $E0
/// 2. Fortress: $67, $EB
/// 3. Page threshold: page0 >= $03, page1 >= $67, page2 >= $BF, page3 >= $E9
/// 4. Map_Removable_Tiles: $51, $52, $54, $67, $EB, $E4, $56, $9D
pub(super) fn is_completion_unsafe(tile: u8) -> bool {
    const SPECIAL: [u8; 5] = [0x50, 0xE8, 0xE6, 0xBD, 0xE0];
    const REMOVABLE: [u8; 8] = [0x51, 0x52, 0x54, 0x67, 0xEB, 0xE4, 0x56, 0x9D];
    const THRESHOLDS: [u8; 4] = [0x03, 0x67, 0xBF, 0xE9];

    // 0x67/0xEB/0x6A are also caught by the threshold check below, but kept
    // explicit here for readability — fortress tiles are the primary case.
    if SPECIAL.contains(&tile) || tile == 0x67 || tile == 0xEB || tile == 0x6A {
        return true;
    }
    let page = (tile >> 6) as usize;
    if tile >= THRESHOLDS[page] {
        return true;
    }
    REMOVABLE.contains(&tile)
}

/// Collect positions whose tile/slot would be "caught" by the game's
/// completion-check routine — the input to `is_row78_conflict`. This covers
/// both completion-unsafe grid tiles and placed Level/Fortress/BonusGame slots
/// (which will be stamped as completion-unsafe tiles by the writer).
pub(super) fn completable_positions(grid: &Grid, slots: &[SlotAssignment]) -> HashSet<(usize, usize)> {
    let mut set: HashSet<(usize, usize)> = HashSet::new();
    for r in 0..grid.rows() {
        for c in 0..grid.cols {
            if is_completion_unsafe(grid.get(r, c)) {
                set.insert((r, c));
            }
        }
    }
    for s in slots {
        if matches!(
            s.kind,
            SlotKind::Level | SlotKind::Fortress | SlotKind::BonusGame | SlotKind::ToadHouse
        ) {
            set.insert(s.pos);
        }
    }
    set
}

/// Promote HammerBro slots to a target `SlotKind`, distributing picked-up pool
/// entries of the matching `NodeKind` across worlds in proportion to each
/// world's available HammerBro slot count.
///
/// Runs after lock placement so reachability constraints are already satisfied;
/// no-ops when the pickup pool contains no matching entries. `budget_cap`
/// limits how many entries are placed (spades cap at SPADE_BUDGET; toad houses
/// place every entry).
pub(super) fn promote_hb_slots<R: Rng>(
    rom: &Rom,
    worlds: &mut [BuiltWorld],
    data: &OverworldData,
    rng: &mut R,
    matches_source: impl Fn(&NodeKind) -> bool,
    target_kind: SlotKind,
    budget_cap: Option<usize>,
) {
    let mut source_count = data
        .pickup
        .pool
        .iter()
        .filter(|pe| matches_source(&data.catalog.entries[pe.catalog_idx].kind))
        .count();
    if let Some(cap) = budget_cap {
        source_count = source_count.min(cap);
    }
    if source_count == 0 {
        return;
    }

    let mut candidates_by_world: Vec<Vec<(usize, usize)>> = Vec::with_capacity(worlds.len());
    for (wi, w) in worlds.iter().enumerate() {
        let sprite_positions: HashSet<(usize, usize)> = rom_data::read_hb_sprite_positions(rom, wi)
            .into_iter()
            .collect();

        let completable = completable_positions(&w.grid, &w.slots);

        let mut cands: Vec<(usize, usize)> = w
            .slots
            .iter()
            .filter(|s| s.kind == SlotKind::HammerBro)
            .map(|s| s.pos)
            .filter(|p| !sprite_positions.contains(p))
            .filter(|p| !is_row78_conflict(*p, &completable))
            .collect();
        cands.shuffle(rng);
        candidates_by_world.push(cands);
    }

    let total_cands: usize = candidates_by_world.iter().map(|c| c.len()).sum();
    if total_cands == 0 {
        return;
    }

    let target = source_count.min(total_cands);
    let mut budget = vec![0usize; worlds.len()];
    for wi in 0..worlds.len() {
        let frac = candidates_by_world[wi].len() as f64 / total_cands as f64;
        budget[wi] = ((frac * target as f64).round() as usize).min(candidates_by_world[wi].len());
    }

    // Rounding may leave us over/under. Trim or pad until we match `target`.
    let mut allocated: usize = budget.iter().sum();
    while allocated > target {
        let wi = budget
            .iter()
            .enumerate()
            .filter(|&(_, b)| *b > 0)
            .max_by_key(|&(_, b)| *b)
            .map(|(i, _)| i)
            .unwrap();
        budget[wi] -= 1;
        allocated -= 1;
    }
    while allocated < target {
        let wi = (0..worlds.len())
            .filter(|&i| candidates_by_world[i].len() > budget[i])
            .max_by_key(|&i| candidates_by_world[i].len() - budget[i]);
        match wi {
            Some(wi) => {
                budget[wi] += 1;
                allocated += 1;
            }
            None => break,
        }
    }

    for (wi, w) in worlds.iter_mut().enumerate() {
        let chosen: HashSet<(usize, usize)> = candidates_by_world[wi]
            .iter()
            .take(budget[wi])
            .copied()
            .collect();
        for slot in w.slots.iter_mut() {
            if slot.kind == SlotKind::HammerBro && chosen.contains(&slot.pos) {
                slot.kind = target_kind.clone();
            }
        }
    }
}

/// Collect positions that must not be overwritten by level/fort/pipe placement.
///
/// This includes: airship, Bowser, toad house tiles (catalog-based), plus the
/// grid positions of floating map object sprites (hammer bro sprites, piranhas,
/// W8 hand traps) read from the ROM's map object tables.
///
/// HammerBro catalog entries are NOT excluded — those blank tiles are valid
/// placement slots. Only the actual floating sprite positions are excluded
/// because a numbered level tile under a floating sprite looks wrong.
pub(crate) fn fixed_positions_for_world(
    rom: &Rom,
    catalog: &NodeCatalog,
    world_idx: usize,
    shuffle_toad_houses: bool,
    shuffle_hammer_bros: bool,
) -> HashSet<(usize, usize)> {
    let mut fixed = HashSet::new();

    // Airship, Bowser stay at vanilla positions unconditionally. Toad Houses
    // stay pinned only when shuffle_toad_houses is off; when on, the build
    // phase places them at promoted HammerBro slots.
    for entry in &catalog.entries {
        if entry.world_idx != world_idx {
            continue;
        }
        match entry.kind {
            NodeKind::Airship | NodeKind::Bowser => {
                fixed.insert(entry.grid_pos);
            }
            NodeKind::ToadHouse if !shuffle_toad_houses => {
                fixed.insert(entry.grid_pos);
            }
            _ => {}
        }
    }

    // Floating sprite positions from the map object tables. When Hammer Bros
    // are redistributed, their vanilla tiles are freed for placement, so only
    // the non-HB sprites (army, canoe, piranhas) stay protected.
    let sprite_positions = if shuffle_hammer_bros {
        rom_data::read_non_hb_sprite_positions(rom, world_idx)
    } else {
        rom_data::read_map_sprite_positions(rom, world_idx)
    };
    for pos in sprite_positions {
        fixed.insert(pos);
    }

    fixed
}

/// Build the patched grids and fixed-position sets, then derive each world's
/// level capacity. Factored out of [`build`] so the distribution-tuning tests
/// compute capacity exactly as production does.
///
/// `patched_grids` clone the pickup grids and restore the Airship/Bowser/Start
/// tiles (blanked during pickup but kept at their possibly-swapped vanilla
/// positions) so BFS/lock placement sees real connectivity. Capacity is the min
/// of grid-blank room and pointer-table room after reserving pipe endpoints and
/// fortresses — the tighter constraint wins, since assigning more entries than
/// pointer-table slots would leave blank screens.
pub(crate) fn prepare_capacities(
    rom: &Rom,
    catalog: &NodeCatalog,
    pickup: &PickupResult,
    fort_counts: &[usize; 8],
    eights_are_wild: bool,
    shuffle_toad_houses: bool,
    shuffle_hammer_bros: bool,
) -> CapacityPrep {
    let mut patched_grids: Vec<Grid> = Vec::with_capacity(8);
    for wi in 0..8 {
        let mut grid = pickup.worlds[wi].grid.clone();
        // The `8s are Wild` flag rides through every downstream clone so the
        // W8 canoe edges gate correctly without per-call threading.
        grid.eights_are_wild = eights_are_wild;
        for entry in &catalog.entries {
            if entry.world_idx != wi {
                continue;
            }
            if matches!(entry.kind, NodeKind::Airship | NodeKind::Bowser | NodeKind::Start) {
                let (r, c) = entry.grid_pos;
                if r < grid.rows() && c < grid.cols {
                    grid.set(r, c, entry.tile);
                }
            }
        }
        crate::randomize::start_airship_swap::swap_tiles_above(&mut grid, wi, catalog);
        patched_grids.push(grid);
    }

    let fixed_positions: Vec<HashSet<(usize, usize)>> = (0..8)
        .map(|wi| fixed_positions_for_world(rom, catalog, wi, shuffle_toad_houses, shuffle_hammer_bros))
        .collect();

    let mut capacities = [0usize; 8];
    for wi in 0..8 {
        let pipe_endpoints = VANILLA_PIPE_PAIRS[wi] * 2;
        let blanks = find_blank_slots(&patched_grids[wi], &fixed_positions[wi]).len();
        let grid_capacity = blanks.saturating_sub(pipe_endpoints + fort_counts[wi]);
        let ptr_slots = pickup.worlds[wi].pool_indices.len();
        let ptr_capacity = ptr_slots.saturating_sub(pipe_endpoints + fort_counts[wi]);
        capacities[wi] = grid_capacity.min(ptr_capacity);
    }

    CapacityPrep {
        patched_grids,
        fixed_positions,
        capacities,
    }
}

/// Distribute `total` levels across worlds by compressed capacity.
///
/// Each world's weight is `capacity^exponent` (see [`LEVEL_SPREAD_EXPONENT`]);
/// the floor of each world's fair share is assigned first (clamped to its hard
/// capacity), then the leftover from flooring is handed out one level at a time
/// to uniformly random worlds that still have spare capacity. The random
/// leftover keeps a little per-seed jitter rather than always topping up the
/// same worlds. A world's count never exceeds its capacity; if every world is
/// at capacity the remainder is dropped (cannot happen for the vanilla total,
/// whose capacity headroom is large).
pub(crate) fn distribute_levels<R: Rng>(
    capacities: &[usize; 8],
    total: usize,
    exponent: f64,
    rng: &mut R,
) -> [usize; 8] {
    let mut counts = [0usize; 8];
    if total == 0 {
        return counts;
    }

    let weights: [f64; 8] = std::array::from_fn(|wi| {
        if capacities[wi] == 0 {
            0.0
        } else {
            (capacities[wi] as f64).powf(exponent)
        }
    });
    let total_w: f64 = weights.iter().sum();
    if total_w == 0.0 {
        return counts;
    }

    // Floor of each world's fair share, clamped to hard capacity.
    for wi in 0..8 {
        let share = weights[wi] / total_w * total as f64;
        counts[wi] = (share.floor() as usize).min(capacities[wi]);
    }

    // Hand out the flooring leftover to random worlds with spare capacity.
    let mut remaining = total.saturating_sub(counts.iter().sum());
    while remaining > 0 {
        let eligible: Vec<usize> = (0..8).filter(|&wi| counts[wi] < capacities[wi]).collect();
        if eligible.is_empty() {
            break;
        }
        let wi = eligible[rng.random_range(..eligible.len())];
        counts[wi] += 1;
        remaining -= 1;
    }

    counts
}

/// Distribute 13 fortresses across W1-W7 (each gets 1-3), W8 keeps 4.
pub(crate) fn redistribute_fortresses<R: Rng>(rng: &mut R) -> [usize; 8] {
    let mut counts = [0usize; 8];
    counts[7] = 4; // W8 always keeps 4

    // Start each of W1-W7 with 1 fortress (= 7 used), leaving 6 to distribute
    for c in counts[..7].iter_mut() {
        *c = 1;
    }
    let mut remaining = 6; // 13 - 7

    // Distribute remaining fortresses randomly, respecting max of 3 per world
    while remaining > 0 {
        let eligible: Vec<usize> = (0..7).filter(|&w| counts[w] < 3).collect();
        if eligible.is_empty() {
            break;
        }
        let &w = eligible.choose(rng).unwrap();
        counts[w] += 1;
        remaining -= 1;
    }

    counts
}

/// Largest number of Hammer Bro sprites placed in a single world.
pub(super) const MAX_HB_PER_WORLD: usize = 3;

/// Map-object slots kept empty in every world so a level-triggered white
/// mushroom house (and similar runtime bonus spawns) has somewhere to appear.
/// White houses are tied to the level, not the world, and levels shuffle across
/// worlds, so the buffer applies everywhere. Conservative: harmless if the
/// engine actually spawns bonuses into the runtime-only slots above the table.
pub(crate) const RESERVED_DYNAMIC_SLOTS: usize = 2;

/// Hammer Bro cap for the Dark World (W8). Its map-object table is dominated by
/// the army (and the optional `8s are Wild` canoe), leaving little room — keep
/// it minimal so dynamic spawns always have headroom there.
pub(super) const W8_HB_CAP: usize = 1;

/// Distribute `total` Hammer Bro sprites across the 8 worlds: each world gets
/// 1-3, bounded by `caps` (free map-object slots and available HammerBro
/// tiles). Seeds every world with one (capacity permitting), then hands out the
/// rest at random — mirrors [`redistribute_fortresses`].
pub(super) fn distribute_hb_sprites<R: Rng>(caps: &[usize; 8], total: usize, rng: &mut R) -> [usize; 8] {
    let mut counts = [0usize; 8];
    for wi in 0..8 {
        counts[wi] = caps[wi].min(1);
    }
    let mut remaining = total.saturating_sub(counts.iter().sum());
    while remaining > 0 {
        let eligible: Vec<usize> = (0..8).filter(|&w| counts[w] < caps[w]).collect();
        if eligible.is_empty() {
            break; // not enough capacity to place all of `total`
        }
        let &w = eligible.choose(rng).unwrap();
        counts[w] += 1;
        remaining -= 1;
    }
    counts
}

/// Pick `count` positions from `candidates`, preferring spread: greedily accept
/// a shuffled candidate when it is not adjacent (Chebyshev distance >= 2) to an
/// already-chosen one, then top up from the remainder if spacing left us short.
pub(super) fn pick_spread_positions<R: Rng>(
    candidates: &[(usize, usize)],
    count: usize,
    rng: &mut R,
) -> Vec<(usize, usize)> {
    let count = count.min(candidates.len());
    let mut shuffled = candidates.to_vec();
    shuffled.shuffle(rng);

    let mut chosen: Vec<(usize, usize)> = Vec::with_capacity(count);
    for &pos in &shuffled {
        if chosen.len() == count {
            break;
        }
        let far = chosen
            .iter()
            .all(|&(r, c)| r.abs_diff(pos.0).max(c.abs_diff(pos.1)) >= 2);
        if far {
            chosen.push(pos);
        }
    }
    // Spacing may have rejected too many; top up from the rest.
    for &pos in &shuffled {
        if chosen.len() == count {
            break;
        }
        if !chosen.contains(&pos) {
            chosen.push(pos);
        }
    }
    chosen
}

/// Decide redistributed Hammer Bro sprite positions + rewards for every world.
/// Selects from each world's final `HammerBro` slot tiles (light anti-clump)
/// and pairs each with a reward picked up from the vanilla encounters. Stores
/// the result in `worlds[wi].hb_sprites`; the writer stamps the ROM tables.
pub(super) fn assign_hb_sprites<R: Rng>(
    rom: &Rom,
    pickup: &PickupResult,
    worlds: &mut [BuiltWorld],
    rng: &mut R,
) {
    // Per-world capacity: bounded by the cosmetic max, the free map-object slots
    // (where the ROM can store a sprite), and the HammerBro tiles to spawn on.
    let mut caps = [0usize; 8];
    let mut hb_tiles: Vec<Vec<(usize, usize)>> = Vec::with_capacity(8);
    for wi in 0..8 {
        let tiles: Vec<(usize, usize)> = worlds[wi]
            .slots
            .iter()
            .filter(|s| s.kind == SlotKind::HammerBro)
            .map(|s| s.pos)
            .collect();
        // Leave RESERVED_DYNAMIC_SLOTS empty for runtime bonus spawns.
        let map_slots = rom_data::eligible_hb_map_slots(rom, wi)
            .len()
            .saturating_sub(RESERVED_DYNAMIC_SLOTS);
        let world_max = if wi == 7 { W8_HB_CAP } else { MAX_HB_PER_WORLD };
        caps[wi] = world_max.min(map_slots).min(tiles.len());
        hb_tiles.push(tiles);
    }

    // Place the vanilla number of encounters, clamped to total capacity.
    let total = pickup.hb_reward_pool.len().min(caps.iter().sum());
    let counts = distribute_hb_sprites(&caps, total, rng);

    // Shuffle the reward pool so rewards aren't tied to their vanilla world.
    let mut rewards = pickup.hb_reward_pool.clone();
    rewards.shuffle(rng);
    let mut reward_iter = rewards.into_iter();

    for wi in 0..8 {
        let positions = pick_spread_positions(&hb_tiles[wi], counts[wi], rng);
        worlds[wi].hb_sprites = positions
            .into_iter()
            .map(|grid_pos| HbSprite {
                grid_pos,
                reward: reward_iter.next().unwrap_or(0),
            })
            .collect();
    }
}
