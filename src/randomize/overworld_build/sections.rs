//! Per-world section building: BFS ordering, blank classification, placement.

use super::*;

use super::locks::place_locks;
use super::pipes::{FIXED_PIPE_ENDPOINTS, PIPE_EXCLUDED_POSITIONS, place_pipes, place_spare_pipes};
use super::scoring::{
    FORTRESS_SOFTMAX_T, is_row78_conflict, pick_softmax_by_score, score_candidate,
    score_fortress_candidate,
};
use super::types::{BuiltWorld, SlotAssignment, SlotKind, WorldSlotCounts};

pub(super) fn build_world<R: Rng>(
    world_idx: usize,
    rom: &Rom,
    mut grid: Grid,
    fixed_positions: &HashSet<(usize, usize)>,
    counts: &WorldSlotCounts,
    shuffle_hammer_bros: bool,
    rng: &mut R,
) -> BuiltWorld {
    let fort_count = counts.fort_count;
    let level_count = counts.level_count;
    let pipe_pair_count = counts.pipe_pair_count;
    let max_non_pipe_slots = counts.max_non_pipe_slots;
    let force_safe = counts.force_safe;
    let start_pos = rom_data::find_start(&grid);
    let target_pos = find_target(&grid, world_idx);

    // Collect all blank node positions (potential placement slots).
    let blank_positions = find_blank_slots(&grid, fixed_positions);

    // Collect fixed pipe endpoints for this world.
    let fixed_pipe_eps: Vec<(usize, usize)> = FIXED_PIPE_ENDPOINTS
        .iter()
        .filter(|(wi, _)| *wi == world_idx)
        .map(|(_, pos)| *pos)
        .collect();

    // Exclude certain positions from pipe placement (unreachable blanks).
    let pipe_excluded: HashSet<(usize, usize)> = PIPE_EXCLUDED_POSITIONS
        .iter()
        .filter(|(wi, _)| *wi == world_idx)
        .map(|(_, pos)| *pos)
        .collect();
    let pipe_blanks: Vec<(usize, usize)> = blank_positions
        .iter()
        .copied()
        .filter(|p| !pipe_excluded.contains(p))
        .collect();

    // Step 1: Place connectivity pipes only (island access + target
    // reachability). Spare pipes are deferred to Step 3.5, after levels exist.
    let mut pipe_pairs = place_pipes(
        &mut grid,
        &pipe_blanks,
        start_pos,
        target_pos,
        pipe_pair_count,
        &fixed_pipe_eps,
        world_idx,
        rng,
    );

    // Collect positions used by the connectivity pipes. Spare pipe positions
    // don't exist yet, so sectioning and level scoring see only these — which
    // is intended: levels are placed as if there were no shortcuts, then the
    // spare pipes are added to skip some of them.
    let pipe_positions: HashSet<(usize, usize)> = pipe_pairs
        .iter()
        .flat_map(|&(a, b)| vec![a, b])
        .collect();

    // Step 2: BFS sectioning
    let sections = bfs_section(
        &grid,
        &pipe_pairs,
        start_pos,
        &blank_positions,
        &pipe_positions,
        fixed_positions,
        fort_count,
        world_idx,
    );

    // Build BFS distance map for scoring — reflects actual walkable distance.
    let bfs_distances: HashMap<(usize, usize), usize> =
        bfs_ordered(&grid, &pipe_pairs, start_pos, world_idx)
            .into_iter()
            .collect();

    // Reverse BFS from target (airship/Bowser) — used to compute path relevance
    // for level scoring. Positions on the main start→target trunk have low detour.
    let reverse_bfs: HashMap<(usize, usize), usize> = target_pos
        .map(|tp| walk_map(&grid, &pipe_pairs, Some(tp), world_idx).distances)
        .unwrap_or_default();
    let target_bfs_dist = target_pos.and_then(|tp| bfs_distances.get(&tp).copied());

    // Step 3: Populate sections
    let mut slots = populate_sections(&grid, &sections, fort_count, level_count, &pipe_positions, &bfs_distances, &reverse_bfs, target_bfs_dist, world_idx, rng);

    // Add mandatory HammerBro slots for HB sprite starting positions.
    // These were excluded from find_blank_slots (so levels/forts/pipes
    // aren't placed under sprites) but still need pointer table entries —
    // the sprite starts there and can be encountered immediately.
    // When shuffle_hammer_bros is on, the vanilla sprite positions are not
    // protected and not forced here — redistribution picks new positions from
    // this world's HammerBro slots after the per-world loop.
    let hb_sprite_pos_list: Vec<(usize, usize)> = if shuffle_hammer_bros {
        Vec::new()
    } else {
        let existing: HashSet<(usize, usize)> = slots.iter().map(|s| s.pos).collect();
        rom_data::read_hb_sprite_positions(rom, world_idx)
            .into_iter()
            .filter(|pos| !existing.contains(pos))
            .collect()
    };
    let hb_sprite_positions: HashSet<(usize, usize)> = hb_sprite_pos_list.iter().copied().collect();
    for pos in &hb_sprite_pos_list {
        slots.push(SlotAssignment {
            pos: *pos,
            kind: SlotKind::HammerBro,
            section: 0,
            is_hand_trap: false,
            is_troll_pipe: false,
        });
    }

    // Cap total slots to what the pointer table can hold. Forts and levels
    // are already within budget (capped during capacity calculation); any
    // excess is purely HammerBro slots from blank tiles. Drop the farthest
    // regular HB slots but never drop HB sprite positions — those are
    // mandatory.
    if slots.len() > max_non_pipe_slots {
        let mut kept: Vec<SlotAssignment> = Vec::with_capacity(max_non_pipe_slots);
        let mut hb_slots: Vec<SlotAssignment> = Vec::new();
        for s in slots {
            if s.kind != SlotKind::HammerBro || hb_sprite_positions.contains(&s.pos) {
                kept.push(s);
            } else {
                hb_slots.push(s);
            }
        }
        let hb_budget = max_non_pipe_slots.saturating_sub(kept.len());
        kept.extend(hb_slots.into_iter().take(hb_budget));
        slots = kept;
    }

    // Step 3.5: Spare pipes. Now that levels are placed, fill the remaining
    // pipe budget by converting HammerBro filler slots into pipe endpoints
    // aimed to skip levels. Runs before locks so `place_locks` accounts for
    // every pipe (a post-lock pipe could otherwise teleport across a lock).
    let spare_needed = pipe_pair_count.saturating_sub(pipe_pairs.len());
    place_spare_pipes(
        &mut grid,
        &mut slots,
        &mut pipe_pairs,
        spare_needed,
        &hb_sprite_positions,
        start_pos,
        world_idx,
        rng,
    );

    // Step 4: Lock placement
    let locks = place_locks(
        &grid,
        &pipe_pairs,
        start_pos,
        target_pos,
        &slots,
        fort_count,
        force_safe,
        world_idx,
        rng,
    );

    BuiltWorld {
        world_idx,
        grid,
        slots,
        locks,
        section_count: fort_count,
        pipe_pairs,
        // Filled in after the per-world loop when shuffle_hammer_bros is on.
        hb_sprites: Vec::new(),
    }
}

/// Find all blank node slots on the grid (positions with theme-blank tiles).
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

/// BFS-ordered list of (position, distance) using the canonical `walk_map`.
///
/// This is the single source of truth for map traversal — all BFS-dependent
/// logic (sectioning, scoring, connectivity checks) must go through here or
/// `walk_map` directly to stay in sync with canoe edges, pipe teleports, etc.
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

/// Split blank positions into (reachable, unreachable) relative to BFS walk,
/// excluding already-used positions.
pub(super) fn split_blanks_by_reachability(
    blanks: &[Pos],
    reachable: &HashSet<Pos>,
    used: &HashSet<Pos>,
) -> (Vec<Pos>, Vec<Pos>) {
    let mut reach = Vec::new();
    let mut unreach = Vec::new();
    for &p in blanks {
        if used.contains(&p) {
            continue;
        }
        if reachable.contains(&p) {
            reach.push(p);
        } else {
            unreach.push(p);
        }
    }
    (reach, unreach)
}

/// Divide reachable blank slots into N sections by BFS distance from start.
// Reason: arguments are distinct traversal inputs (grid, pipes, start, blanks,
// pipe/fixed position sets, section count, world); they don't cluster into a
// meaningful concept, so a bundling struct would be a lint bandage, not a real
// abstraction.
#[allow(clippy::too_many_arguments)]
pub(super) fn bfs_section(
    grid: &Grid,
    pipe_pairs: &[TeleportEdge],
    start_pos: Option<(usize, usize)>,
    blank_positions: &[(usize, usize)],
    pipe_positions: &HashSet<(usize, usize)>,
    fixed_positions: &HashSet<(usize, usize)>,
    section_count: usize,
    world_idx: usize,
) -> Vec<Vec<(usize, usize)>> {
    if section_count == 0 {
        return vec![blank_positions
            .iter()
            .copied()
            .filter(|p| !pipe_positions.contains(p))
            .collect()];
    }

    // BFS-order all reachable positions
    let ordered = bfs_ordered(grid, pipe_pairs, start_pos, world_idx);

    // Filter to only blank slots that aren't used by pipes or fixed entries
    let assignable: Vec<(usize, usize)> = ordered
        .iter()
        .map(|&(pos, _)| pos)
        .filter(|p| {
            blank_positions.contains(p)
                && !pipe_positions.contains(p)
                && !fixed_positions.contains(p)
        })
        .collect();

    if assignable.is_empty() {
        return vec![assignable];
    }

    // Divide into roughly equal sections
    let per_section = assignable.len() / section_count;
    let extra = assignable.len() % section_count;

    let mut sections = Vec::with_capacity(section_count);
    let mut offset = 0;
    for i in 0..section_count {
        let size = per_section + if i < extra { 1 } else { 0 };
        sections.push(assignable[offset..offset + size].to_vec());
        offset += size;
    }

    sections
}

// Reason: 10 args is over clippy's 7-arg default. Candidate bundles
// investigated (`BfsCtx` for the 3 distance args, reusing `WorldSlotCounts`
// for the 2 budget args) — none reveals a concept beyond what the inline
// arg names already convey. Each arg is a distinct input (geometry,
// sections, budgets, pipe positions, BFS data, world, RNG) and bundling
// them would add indirection without clarity.
#[allow(clippy::too_many_arguments)]
pub(super) fn populate_sections<R: Rng>(
    grid: &Grid,
    sections: &[Vec<(usize, usize)>],
    fort_count: usize,
    level_count: usize,
    pipe_positions: &HashSet<(usize, usize)>,
    bfs_distances: &HashMap<(usize, usize), usize>,
    reverse_bfs: &HashMap<(usize, usize), usize>,
    target_bfs_dist: Option<usize>,
    world_idx: usize,
    rng: &mut R,
) -> Vec<SlotAssignment> {
    let mut slots = Vec::new();

    // Two separate sets:
    // 1. `completable` — all completion-unsafe tiles on the grid. Used for
    //    the row 7/8 hard constraint (game engine bug). Includes spades,
    //    airships, etc.
    // 2. `placed_levels_and_forts` — only levels and fortresses we've placed.
    //    Used by the scoring function to spread levels apart. Excludes spades,
    //    pipes, and other non-clumping tiles.
    let mut completable = completable_positions(grid, &[]);
    let mut placed_levels_and_forts: HashSet<(usize, usize)> = HashSet::new();

    // Add pipe slots (not in sections, but tracked)
    for &pos in pipe_positions {
        slots.push(SlotAssignment {
            pos,
            kind: SlotKind::Pipe,
            section: 0, // pipes don't really belong to a section
            is_hand_trap: false,
            is_troll_pipe: false,
        });
    }

    // Phase 1: Place one fortress per section.
    // Track which positions are fortresses so we exclude them from level candidates.
    let mut fort_positions: HashSet<(usize, usize)> = HashSet::new();

    for (si, section) in sections.iter().enumerate() {
        if section.is_empty() || si >= fort_count {
            continue;
        }

        // Score all candidates in this section, filtering row 7/8 conflicts.
        let candidates: Vec<((usize, usize), f64)> = section
            .iter()
            .filter(|pos| !is_row78_conflict(**pos, &completable))
            .map(|&pos| {
                (pos, score_fortress_candidate(grid, pos, &placed_levels_and_forts, bfs_distances, world_idx))
            })
            .collect();

        // Sample by softmax; fallback to any section slot if none passed the row78 filter.
        let pos = pick_softmax_by_score(candidates, FORTRESS_SOFTMAX_T, rng)
            .unwrap_or_else(|| section[rng.random_range(..section.len())]);

        completable.insert(pos);
        placed_levels_and_forts.insert(pos);
        fort_positions.insert(pos);
        slots.push(SlotAssignment {
            pos,
            kind: SlotKind::Fortress,
            section: si,
            is_hand_trap: false,
            is_troll_pipe: false,
        });
    }

    // Phase 2: Place levels globally across all sections using score-based
    // picking. Candidates are all non-fortress positions from every section.
    let mut global_candidates: Vec<((usize, usize), usize)> = Vec::new(); // (pos, section_idx)
    for (si, section) in sections.iter().enumerate() {
        for &pos in section {
            if !fort_positions.contains(&pos) {
                global_candidates.push((pos, si));
            }
        }
    }

    let mut level_positions: HashSet<(usize, usize)> = HashSet::new();

    for _ in 0..level_count {
        // Score each candidate once, then pick the max on the cached score.
        // (`max_by` returns the LAST maximal element on ties, matching the
        // pre-caching behavior.)
        let best = global_candidates
            .iter()
            .filter(|(pos, _)| !level_positions.contains(pos))
            .filter(|(pos, _)| !is_row78_conflict(*pos, &completable))
            .map(|&(pos, _)| {
                let score = score_candidate(grid, pos, &placed_levels_and_forts, bfs_distances, reverse_bfs, target_bfs_dist);
                (pos, score)
            })
            .max_by(|(_, sa), (_, sb)| sa.partial_cmp(sb).unwrap_or(std::cmp::Ordering::Equal));

        match best {
            Some((pos, _)) => {
                level_positions.insert(pos);
                completable.insert(pos);
                placed_levels_and_forts.insert(pos);
            }
            None => break,
        }
    }

    // Phase 3: Emit remaining slots — levels and hammer bros.
    for (si, section) in sections.iter().enumerate() {
        for &pos in section {
            if fort_positions.contains(&pos) {
                continue; // already emitted in phase 1
            }
            if level_positions.contains(&pos) {
                slots.push(SlotAssignment {
                    pos,
                    kind: SlotKind::Level,
                    section: si,
                    is_hand_trap: false,
                    is_troll_pipe: false,
                });
            } else {
                slots.push(SlotAssignment {
                    pos,
                    kind: SlotKind::HammerBro,
                    section: si,
                    is_hand_trap: false,
                    is_troll_pipe: false,
                });
            }
        }
    }

    slots
}
