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
use super::route_choice::{
    COST_LEVEL, DEFAULT_SLACK, SHAPING_SLACK, analyze_route_choice, in_band_count, measure_counts,
};
use super::scoring::{is_row78_conflict, score_candidate};
use super::shape::{enforce_c1_floor, shape_forts};
use super::types::{BuiltWorld, SlotAssignment, SlotKind, WorldSlotCounts, hb_slot_index};

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
        world_idx,
        &grid,
        &assignable,
        level_count,
        &pipe_positions,
        &pipe_pairs,
        &bfs_distances,
        &reverse_bfs,
        target_bfs_dist,
        &knobs.level,
        rng,
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

    // Step 5.2: C1 floor. With the cost landscape final (locks committed),
    // ensure the cheapest route charges at least C1_FLOOR points where the
    // content budget allows — off-route levels migrate onto the cheap route.
    // Level moves never change walkability, so the locks stay valid.
    enforce_c1_floor(&mut built, &hb_sprite_positions);

    // Reserve one pipe from the spare budget for a possible compound gated
    // shortcut (Step 5.6). That move must run AFTER the spare-pipe pass — a
    // spare pipe added later can collapse the choice a gate creates — but it
    // still needs a pipe slot to spend, so hold one back here. If the move
    // declines it, it is placed as an ordinary spare below.
    let spare_needed = pipe_pair_count.saturating_sub(built.pipe_pairs.len());
    let hold = spare_needed.min(1);

    // Step 5.5: Spare pipes. Now that levels AND locks exist, fill the
    // remaining pipe budget by converting HammerBro filler slots into pipe
    // endpoints. Each pair is scored lock-aware: at most one pair per world
    // may bypass a single mandatory fortress (the fort-skip prize); pairs
    // that would bypass 2+ forts or open the goal outright are rejected;
    // the rest aim to skip levels as before.
    place_spare_pipes(
        &mut built.grid,
        &mut built.slots,
        &mut built.pipe_pairs,
        spare_needed - hold,
        &hb_sprite_positions,
        &built.locks,
        &knobs.pipe,
        start_pos,
        target_pos,
        world_idx,
        rng,
    );

    // Step 5.6: Compound gated shortcut (issue #125). One joint pipe+lock move
    // on a still-linear world: a content-skipping pipe made fair by relocating
    // an off-route fort's lock onto the shortcut. Runs LAST among the pipe
    // passes so nothing downstream collapses the choice, spending the reserved
    // slot. Applied only if it strictly raises the in-band route count.
    let used = compound::place_gated_shortcut(
        &mut built,
        hold,
        &hb_sprite_positions,
        start_pos,
        target_pos,
        rng,
    );
    if hold > used {
        // Declined — spend the reserved pipe as an ordinary spare.
        place_spare_pipes(
            &mut built.grid,
            &mut built.slots,
            &mut built.pipe_pairs,
            hold - used,
            &hb_sprite_positions,
            &built.locks,
            &knobs.pipe,
            start_pos,
            target_pos,
            world_idx,
            rng,
        );
    }

    // Step 5.7: C1 floor, final pass. The spare-pipe floor guard is SOFT —
    // the vanilla pipe budget outranks it, so a pipe-rich world can be
    // forced to place a floor-violating shortcut (min C1 sank to 4 on W7
    // without this). Repair against the FINAL cost landscape (spares +
    // gated shortcut committed): level moves can't change walkability, so
    // pipes and locks stay valid.
    enforce_c1_floor(&mut built, &hb_sprite_positions);

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

/// Aesthetic-score candidates measured per choice-aware level placement.
const LEVEL_AESTHETIC_CANDIDATES: usize = 5;
/// Cap on total measured candidates per choice-aware level placement.
const LEVEL_MEASURED_CANDIDATES: usize = 8;
/// Detour-derived targets kept per detour (ranked by aesthetic score).
const LEVEL_TARGETS_PER_DETOUR: usize = 2;

/// Place `level_count` levels on the assignable blanks, then emit every slot:
/// pipes, levels, and HammerBro fillers for the remaining blanks.
///
/// Choice-aware split: the FIRST half is placed on the aesthetic score alone
/// (greedy spread + path relevance — the terrain; uniform random instead when
/// `knobs.random_first_half`), the SECOND half is placed greedily too and
/// KEPT when the layout already measures choiceful (lazy repair — one cheap
/// measure instead of ~25); only linear layouts re-place it MEASURED. Per
/// measured level,
/// candidates are the top aesthetic scorers plus detour-derived targets —
/// blanks on the cheap route's exclusive stretch vs each dominated detour
/// whose gap one level can rescue — and the pick maximizes the in-band route
/// count, aesthetic score as tie-break. The point: `path_bonus` glues levels
/// to the trunk, leaving every cycle's short side empty, which reads as a
/// nested dominated detour; ONE level on a node-bearing short side makes the
/// two level-sets disjoint and the detour a real choice. (Zero-node short
/// sides are the golden-lock pass's job; true trees fall through to pure
/// aesthetics.)
///
/// Sections are all 0 — only fortress slots carry a meaningful section index,
/// assigned later by the shaping pass and `renumber_fort_sections`.
// Reason: 10 args is over clippy's 7-arg default. Each is a distinct input
// (geometry, candidate pool, budget, pipe data, BFS data, knobs, RNG);
// bundling any subset would add indirection without clarity.
#[allow(clippy::too_many_arguments)]
fn place_levels<R: Rng>(
    world_idx: usize,
    grid: &Grid,
    assignable: &[(usize, usize)],
    level_count: usize,
    pipe_positions: &HashSet<(usize, usize)>,
    pipe_pairs: &[TeleportEdge],
    bfs_distances: &HashMap<(usize, usize), usize>,
    reverse_bfs: &HashMap<(usize, usize), usize>,
    target_bfs_dist: Option<usize>,
    knobs: &LevelScoring,
    rng: &mut R,
) -> Vec<SlotAssignment> {
    // Two separate sets:
    // 1. `completable` — all completion-unsafe tiles on the grid. Used for
    //    the row 7/8 hard constraint (game engine bug). Includes spades,
    //    airships, etc.
    // 2. `placed_levels` — only levels we've placed. Used by the scoring
    //    function to spread levels apart. Excludes spades, pipes, and other
    //    non-clumping tiles.
    let mut completable = completable_positions(grid, &[]);
    let mut placed_levels: HashSet<(usize, usize)> = HashSet::new();

    let measured_count = level_count / 2;
    let greedy_count = level_count - measured_count;
    // Second-half picks made greedily — kept if the greedy layout already
    // measures choiceful, ripped up and re-placed measured if not.
    let mut greedy_tail: Vec<(usize, usize)> = Vec::new();

    for i in 0..level_count {
        let pick = if knobs.random_first_half {
            let eligible: Vec<(usize, usize)> = assignable
                .iter()
                .copied()
                .filter(|pos| !placed_levels.contains(pos))
                .filter(|pos| !is_row78_conflict(*pos, &completable))
                .collect();
            eligible.choose(rng).copied()
        } else {
            // Score each candidate once, then pick the max on the cached
            // score. (`max_by` returns the LAST maximal element on ties,
            // matching the pre-caching behavior.)
            assignable
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
                .max_by(|(_, sa), (_, sb)| sa.partial_cmp(sb).unwrap_or(std::cmp::Ordering::Equal))
                .map(|(pos, _)| pos)
        };

        match pick {
            Some(pos) => {
                placed_levels.insert(pos);
                completable.insert(pos);
                if i >= greedy_count {
                    greedy_tail.push(pos);
                }
            }
            None => break,
        }
    }

    // Emit pipe slots (already stamped on the grid, tracked as slots), then
    // level and HammerBro filler slots — the measured pass below flips more
    // fillers to levels in place.
    let mut slots: Vec<SlotAssignment> = Vec::new();
    for &pos in pipe_positions {
        slots.push(SlotAssignment {
            pos,
            kind: SlotKind::Pipe,
            section: 0,
            is_hand_trap: false,
            is_troll_pipe: false,
        });
    }
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
    if measured_count == 0 {
        return slots;
    }

    // Interim world for measuring: no forts/locks yet, so routes differ only
    // by level-sets — exactly the structure a level placement can shape.
    // LAZY REPAIR: measure the all-greedy layout once — when it already
    // offers 2+ in-band routes, keep it and skip the measured rework
    // entirely; only linear layouts pay for the per-candidate measures.
    let mut interim = BuiltWorld {
        world_idx,
        grid: grid.clone(),
        slots,
        locks: Vec::new(),
        section_count: 0,
        pipe_pairs: pipe_pairs.to_vec(),
        hb_sprites: Vec::new(),
    };

    let rc0 = measure_counts(&interim, DEFAULT_SLACK);
    if in_band_count(&rc0) >= 2 {
        return interim.slots;
    }
    // Linear — rip up the greedy second half and re-place it measured, from
    // the same first-half-only state the measured pass has always seen.
    for &pos in &greedy_tail {
        if let Some(i) = interim
            .slots
            .iter()
            .position(|s| s.pos == pos && s.kind == SlotKind::Level)
        {
            interim.slots[i].kind = SlotKind::HammerBro;
        }
        placed_levels.remove(&pos);
        completable.remove(&pos);
    }

    for _ in 0..measured_count {
        let rc = analyze_route_choice(&interim, SHAPING_SLACK);

        // Aesthetic scores for every remaining eligible blank, best first.
        let mut scored: Vec<((usize, usize), f64)> = assignable
            .iter()
            .copied()
            .filter(|pos| !placed_levels.contains(pos))
            .filter(|pos| !is_row78_conflict(*pos, &completable))
            .map(|pos| {
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
            .collect();
        scored.sort_by(|a, b| {
            b.1.partial_cmp(&a.1)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.0.cmp(&b.0))
        });
        if scored.is_empty() {
            break;
        }
        let score_of: HashMap<Pos, f64> = scored.iter().copied().collect();

        // Candidates: detour-derived targets first (they're the point), then
        // the top aesthetic scorers as the fallback pool.
        let mut candidates: Vec<Pos> = Vec::new();
        if let Some(cheap) = rc.routes.first() {
            // A detour is level-rescuable when +COST_LEVEL on the cheap route
            // lands its gap inside the choice band.
            let rescuable = COST_LEVEL..=(COST_LEVEL + DEFAULT_SLACK);
            for detour in rc
                .detours
                .iter()
                .filter(|d| rescuable.contains(&(d.cost - rc.best_cost)))
                .take(3)
            {
                let detour_nodes: HashSet<Pos> = detour.path.iter().copied().collect();
                let mut targets: Vec<Pos> = cheap
                    .path
                    .iter()
                    .copied()
                    .filter(|p| !detour_nodes.contains(p))
                    .filter(|p| score_of.contains_key(p))
                    .collect();
                targets.sort_by(|a, b| {
                    score_of[b]
                        .partial_cmp(&score_of[a])
                        .unwrap_or(std::cmp::Ordering::Equal)
                        .then_with(|| a.cmp(b))
                });
                candidates.extend(targets.into_iter().take(LEVEL_TARGETS_PER_DETOUR));
            }
        }
        candidates.extend(scored.iter().take(LEVEL_AESTHETIC_CANDIDATES).map(|&(p, _)| p));
        let mut seen: HashSet<Pos> = HashSet::new();
        candidates.retain(|p| seen.insert(*p));
        candidates.truncate(LEVEL_MEASURED_CANDIDATES);

        // Measure each candidate; keep the max (in-band routes, aesthetic
        // score). First-in-candidate-order wins ties, so detour targets beat
        // equal aesthetic picks.
        let mut best: Option<(Pos, usize, f64)> = None;
        for &pos in &candidates {
            let Some(idx) = hb_slot_index(&interim, pos) else { continue };
            interim.slots[idx].kind = SlotKind::Level;
            // Trial: only `in_band_count` is read, defined on the
            // `DEFAULT_SLACK` band — measure narrow, no paths.
            let rc2 = measure_counts(&interim, DEFAULT_SLACK);
            interim.slots[idx].kind = SlotKind::HammerBro;
            let (in_band, score) = (in_band_count(&rc2), score_of[&pos]);
            if best.is_none_or(|(_, bi, bs)| in_band > bi || (in_band == bi && score > bs)) {
                best = Some((pos, in_band, score));
            }
        }
        let Some((pos, _, _)) = best else { break };
        let Some(idx) = hb_slot_index(&interim, pos) else { break };
        interim.slots[idx].kind = SlotKind::Level;
        placed_levels.insert(pos);
        completable.insert(pos);
    }

    interim.slots
}
