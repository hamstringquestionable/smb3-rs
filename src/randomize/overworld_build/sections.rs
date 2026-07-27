//! Per-world building: BFS ordering, blank classification, placement.
//!
//! Choice-first pipeline order: connectivity pipes → levels (the terrain) →
//! mandatory HB slots + pointer-budget cap → measured fort shaping
//! (`shape_forts`) → fort section renumbering by BFS rank → locks → spare
//! pipes.

use super::*;

use super::knobs::{Knobs, LevelScoring};
use super::locks::place_locks;
use super::pipes::{FIXED_PIPE_ENDPOINTS, PIPE_EXCLUDED_POSITIONS, place_pipes, place_spare_pipes};
use super::scoring::{is_row78_conflict, score_candidate};
use super::shape::shape_forts;
use super::types::{BuiltWorld, SlotAssignment, SlotKind, WorldSlotCounts};

// Reason: each arg is a distinct build input (world, rom, grid, fixed slots,
// budgets, knobs, HB flag, RNG). No subset forms a cohesive concept worth
// a struct — bundling would add indirection without clarifying anything.
#[allow(clippy::too_many_arguments)]
pub(super) fn build_world<R: Rng>(
    world_idx: usize,
    rom: &Rom,
    mut grid: Grid,
    fixed_positions: &HashSet<(usize, usize)>,
    counts: &WorldSlotCounts,
    knobs: &Knobs,
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
    let pipe_pairs = place_pipes(
        &mut grid,
        &pipe_blanks,
        start_pos,
        target_pos,
        pipe_pair_count,
        &fixed_pipe_eps,
        &knobs.pipe,
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

    // Step 2: BFS-ordered assignable blanks (reachable, not a pipe endpoint,
    // not a fixed entry). This is the candidate pool for levels and — via
    // the HammerBro fillers they become — forts.
    let ordered = bfs_ordered(&grid, &pipe_pairs, start_pos, world_idx);
    let bfs_distances: HashMap<(usize, usize), usize> = ordered.iter().copied().collect();
    let assignable: Vec<(usize, usize)> = ordered
        .iter()
        .map(|&(pos, _)| pos)
        .filter(|p| {
            blank_positions.contains(p)
                && !pipe_positions.contains(p)
                && !fixed_positions.contains(p)
        })
        .collect();

    // Reverse BFS from target (airship/Bowser) — used to compute path relevance
    // for level scoring. Positions on the main start→target trunk have low detour.
    let reverse_bfs: HashMap<(usize, usize), usize> = target_pos
        .map(|tp| walk_map(&grid, &pipe_pairs, Some(tp), world_idx).distances)
        .unwrap_or_default();
    let target_bfs_dist = target_pos.and_then(|tp| bfs_distances.get(&tp).copied());

    // Step 3: Levels first (the terrain forts respond to), remaining blanks
    // become HammerBro fillers — the conversion stock for forts and spare
    // pipes.
    let mut slots = place_levels(
        &grid,
        &assignable,
        level_count,
        &pipe_positions,
        &bfs_distances,
        &reverse_bfs,
        target_bfs_dist,
        &knobs.level,
    );

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

    // Cap total slots to what the pointer table can hold. Levels are
    // already within budget (capped during capacity calculation, which also
    // reserves fort_count on top); any excess is purely HammerBro slots from
    // blank tiles. Drop the farthest regular HB slots but never drop HB
    // sprite positions — those are mandatory. Runs BEFORE fort shaping: the
    // HB→Fortress conversion doesn't change the slot count, and the reserved
    // fort headroom means the cap always leaves >= fort_count HB slots.
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

    // Step 4: Measured fort shaping — convert HammerBro fillers to
    // fortresses, using route measurement to create route choice where the
    // terrain allows it. Returns the gate-fort hint for the lock pass.
    let mut built = BuiltWorld {
        world_idx,
        grid,
        slots,
        locks: Vec::new(),
        section_count: fort_count,
        pipe_pairs,
        // Filled in after the per-world loop when shuffle_hammer_bros is on.
        hb_sprites: Vec::new(),
    };
    let gate_pos = shape_forts(
        &mut built,
        fort_count,
        &hb_sprite_positions,
        &bfs_distances,
        &knobs.fort,
        rng,
    );
    // Renumber fort sections to BFS-distance rank: the lock pass simulates
    // progression in section order, and downstream (assignment, fortress FX)
    // pairs locks to forts by section index.
    let gate_section = renumber_fort_sections(&mut built.slots, &bfs_distances, gate_pos);

    // Step 5: Lock placement. Runs BEFORE the spare-pipe pass so locks are
    // placed on pure geography (connectivity pipes only) — the spare pipes
    // below may then deliberately bridge across ONE of them (a fort skip).
    built.locks = place_locks(
        &built.grid,
        &built.pipe_pairs,
        start_pos,
        target_pos,
        &built.slots,
        fort_count,
        &knobs.lock,
        gate_section,
        force_safe,
        world_idx,
        rng,
    );

    // Step 5.5: Spare pipes. Now that levels AND locks exist, fill the
    // remaining pipe budget by converting HammerBro filler slots into pipe
    // endpoints. Each pair is scored lock-aware: at most one pair per world
    // may bypass a single mandatory fortress (the fort-skip prize); pairs
    // that would bypass 2+ forts or open the goal outright are rejected;
    // the rest aim to skip levels as before.
    let spare_needed = pipe_pair_count.saturating_sub(built.pipe_pairs.len());
    place_spare_pipes(
        &mut built.grid,
        &mut built.slots,
        &mut built.pipe_pairs,
        spare_needed,
        &hb_sprite_positions,
        &built.locks,
        &knobs.pipe,
        start_pos,
        target_pos,
        world_idx,
        rng,
    );

    built
}

/// Renumber fortress sections to BFS-distance rank (position tie-break) and
/// map the gate-fort position hint to its final section index. The lock
/// pass's progression simulation ("earlier sections beaten first") needs
/// sections ordered by how soon the player can reach each fort.
fn renumber_fort_sections(
    slots: &mut [SlotAssignment],
    bfs_distances: &HashMap<(usize, usize), usize>,
    gate_pos: Option<(usize, usize)>,
) -> Option<usize> {
    let mut forts: Vec<(usize, (usize, usize))> = slots
        .iter()
        .enumerate()
        .filter(|(_, s)| s.kind == SlotKind::Fortress)
        .map(|(i, s)| (i, s.pos))
        .collect();
    forts.sort_by_key(|&(_, pos)| (bfs_distances.get(&pos).copied().unwrap_or(usize::MAX), pos));

    let mut gate_section = None;
    for (rank, &(idx, pos)) in forts.iter().enumerate() {
        slots[idx].section = rank;
        if gate_pos == Some(pos) {
            gate_section = Some(rank);
        }
    }
    gate_section
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

/// Place `level_count` levels on the assignable blanks by greedy score-based
/// picking (no RNG), then emit every slot: pipes, levels, and HammerBro
/// fillers for the remaining blanks. Sections are all 0 — only fortress
/// slots carry a meaningful section index, assigned later by the shaping
/// pass and `renumber_fort_sections`.
// Reason: 8 args is over clippy's 7-arg default. Each is a distinct input
// (geometry, candidate pool, budget, pipe positions, BFS data, knobs);
// bundling any subset would add indirection without clarity.
#[allow(clippy::too_many_arguments)]
fn place_levels(
    grid: &Grid,
    assignable: &[(usize, usize)],
    level_count: usize,
    pipe_positions: &HashSet<(usize, usize)>,
    bfs_distances: &HashMap<(usize, usize), usize>,
    reverse_bfs: &HashMap<(usize, usize), usize>,
    target_bfs_dist: Option<usize>,
    knobs: &LevelScoring,
) -> Vec<SlotAssignment> {
    let mut slots = Vec::new();

    // Two separate sets:
    // 1. `completable` — all completion-unsafe tiles on the grid. Used for
    //    the row 7/8 hard constraint (game engine bug). Includes spades,
    //    airships, etc.
    // 2. `placed_levels` — only levels we've placed. Used by the scoring
    //    function to spread levels apart. Excludes spades, pipes, and other
    //    non-clumping tiles.
    let mut completable = completable_positions(grid, &[]);
    let mut placed_levels: HashSet<(usize, usize)> = HashSet::new();

    // Pipe slots (already stamped on the grid, tracked as slots).
    for &pos in pipe_positions {
        slots.push(SlotAssignment {
            pos,
            kind: SlotKind::Pipe,
            section: 0,
            is_hand_trap: false,
            is_troll_pipe: false,
        });
    }

    for _ in 0..level_count {
        // Score each candidate once, then pick the max on the cached score.
        // (`max_by` returns the LAST maximal element on ties, matching the
        // pre-caching behavior.)
        let best = assignable
            .iter()
            .filter(|pos| !placed_levels.contains(*pos))
            .filter(|pos| !is_row78_conflict(**pos, &completable))
            .map(|&pos| {
                let score = score_candidate(
                    grid,
                    pos,
                    &placed_levels,
                    bfs_distances,
                    reverse_bfs,
                    target_bfs_dist,
                    knobs,
                );
                (pos, score)
            })
            .max_by(|(_, sa), (_, sb)| sa.partial_cmp(sb).unwrap_or(std::cmp::Ordering::Equal));

        match best {
            Some((pos, _)) => {
                placed_levels.insert(pos);
                completable.insert(pos);
            }
            None => break,
        }
    }

    // Emit level and HammerBro filler slots.
    for &pos in assignable {
        slots.push(SlotAssignment {
            pos,
            kind: if placed_levels.contains(&pos) {
                SlotKind::Level
            } else {
                SlotKind::HammerBro
            },
            section: 0,
            is_hand_trap: false,
            is_troll_pipe: false,
        });
    }

    slots
}
