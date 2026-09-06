//! The maze generator's own tests, and its censuses.
//!
//! Two kinds of thing live here and they are not the same kind. The `#[test]`s
//! assert properties a failure of which is a bug — the maze is finishable, no
//! world can strand a player, every pad is half of a pair. The `#[ignore]`d
//! censuses measure shape, have no target numbers, and exist so a knob can be
//! argued as a delta against something.
//!
//! The null-model census that opened this file is gone with the uniform placer
//! it measured; see the note in the parent module.

use rand::SeedableRng;
use rand::seq::SliceRandom;
use rand_chacha::ChaCha8Rng;

use super::graph::{Knobs, PAD_BUDGET};
use super::{GenReport, GlobalState, IDENTITY_SPINE, MazeEdge};
use crate::randomize::map_walker::walk_reachable;
use crate::randomize::maze::walk::{MazeWorld, walk_maze};
use crate::randomize::node_catalog::NodeCatalog;
use crate::randomize::overworld_build::{
    BuildFlags, BuildResult, OverworldData, SlotKind, build, stamp_slots,
};
use crate::randomize::overworld_pickup::{PickupFlags, pick_up};
use crate::randomize::rom_data::Grid;
use crate::randomize::{qol, start_airship_swap};
use crate::rom::Rom;

fn load_rom() -> Option<Rom> {
    let bytes = std::fs::read("roms/Super Mario Bros. 3 (USA) (Rev 1).nes").ok()?;
    Rom::from_bytes(&bytes).ok()
}

fn census_seeds(default: u64) -> u64 {
    std::env::var("CENSUS_SEEDS").ok().and_then(|s| s.parse().ok()).unwrap_or(default)
}

/// Map QOL in production order, per census arm — a copy of the overworld
/// census's `apply_qol_variant`, which is private to that test module.
fn qol_variant(rom: &Rom, hammer_rocks: bool, eights_wild: bool) -> Rom {
    let mut out = rom.clone();
    qol::fix_w3_drawbridges(&mut out);
    qol::remove_rocks(&mut out);
    if hammer_rocks {
        qol::make_hammer_rocks(&mut out);
    }
    qol::apply_w1_shortcut(&mut out, hammer_rocks);
    qol::apply_w8_bridges(&mut out);
    if eights_wild {
        qol::apply_w8_canoe_and_paths(&mut out);
    }
    qol::fix_big_q_block_rooms(&mut out);
    out
}

/// One seed's eight-world build, over the same flag arms the overworld census
/// uses: 50% base / 25% more-hammer-rocks / 25% 8s-are-wild, with start↔airship
/// swap rolled per world. The maze must hold on every map the builder can
/// produce, not on one arm of it.
fn census_build(raw: &Rom, seed: u64) -> (Rom, BuildResult) {
    census_build_swaps(raw, seed).0
}

/// The same, also reporting which worlds start↔airship swap took — so a test
/// can see how much of that arm a seed actually exercised instead of assuming.
fn census_build_swaps(raw: &Rom, seed: u64) -> ((Rom, BuildResult), [bool; 8]) {
    let (hammer_rocks, eights_wild) = match seed % 4 {
        2 => (true, false),
        3 => (false, true),
        _ => (false, false),
    };
    let rom = qol_variant(raw, hammer_rocks, eights_wild);
    let mut rng = ChaCha8Rng::seed_from_u64(seed);
    let mut catalog = NodeCatalog::build(&rom, false);
    let mut swap_rng = ChaCha8Rng::seed_from_u64(seed);
    start_airship_swap::pick_swaps(&mut catalog, &mut swap_rng);
    let pickup = pick_up(
        &rom,
        &catalog,
        PickupFlags { shuffle_spade_games: true, shuffle_toad_houses: true, ..Default::default() },
    );
    let result = build(
        &rom,
        &OverworldData { pickup: &pickup, catalog: &catalog },
        &mut rng,
        BuildFlags {
            shuffle_toad_houses: true,
            eights_are_wild: eights_wild,
            ..Default::default()
        },
    );
    let swaps = catalog.start_airship_swapped;
    ((rom, result), swaps)
}

/// One generated maze, and the ROM its eight worlds were built from — which
/// the tests that read a stamped map back need beside the state.
///
/// This is the shape everything below measures: the identity spine, the real
/// pad placer, the real key fill. There is no second placer to compare it to
/// any more (see the parent module's note on the null model).
fn generated(raw: &Rom, seed: u64, knobs: &Knobs, k: u8) -> (Rom, GlobalState, GenReport) {
    let (rom, result) = census_build(raw, seed);
    let mut rng = ChaCha8Rng::seed_from_u64(seed ^ 0x5EED_1234);
    let (state, report) = super::generate(&result, &IDENTITY_SPINE, k, knobs, &mut rng);
    (rom, state, report)
}

// ---------------------------------------------------------------------------
// The walker
// ---------------------------------------------------------------------------

/// The maze walker is a COPY of `reach_from`'s 2-tile move expansion (see the
/// note above it), and this is what stops the copy drifting: given the eight
/// worlds and no links, walking from one world's start must reproduce
/// `walk_reachable` for that world cell for cell — and reach nothing at all in
/// the other seven, since without a link there is no way across.
///
/// The oracle is the shipping walker rather than a hand-written expectation,
/// which is the point: a fixture that shares the code's assumption cannot test
/// that assumption.
#[test]
fn maze_walk_matches_the_per_world_walker() {
    let Some(raw) = load_rom() else { return };
    for seed in 0..census_seeds(4) {
        let (_, result) = census_build(&raw, seed);
        let state = GlobalState::from_build(&result, &IDENTITY_SPINE, 0);

        // Locks closed, slots stamped — the fixpoint's round-zero grids.
        let grids: Vec<Grid> = state
            .worlds
            .iter()
            .map(|w| {
                let mut g = w.grid.clone();
                stamp_slots(&mut g, &w.slots);
                for lock in &w.locks {
                    g.set(lock.pos.0, lock.pos.1, lock.gap_tile);
                }
                g
            })
            .collect();
        let view: Vec<MazeWorld> = grids
            .iter()
            .zip(&state.worlds)
            .map(|(grid, w)| MazeWorld { grid, pipe_pairs: &w.pipe_pairs })
            .collect();

        for (wi, grid) in grids.iter().enumerate() {
            let w = &state.worlds[wi];
            let want = walk_reachable(grid, &w.pipe_pairs, w.start, wi);
            let got = walk_maze(&view, &[], (wi, w.start.expect("every world has a START")));
            for r in 0..grid.rows() {
                for c in 0..grid.cols {
                    assert_eq!(
                        want.contains((r, c)),
                        got.contains((wi, (r, c))),
                        "seed {seed} W{} ({r},{c}): maze walker disagrees with walk_reachable",
                        wi + 1
                    );
                }
            }
            for other in (0..8).filter(|&o| o != wi) {
                assert_eq!(
                    got.world_len(other),
                    0,
                    "seed {seed}: walking W{} with no links reached W{}",
                    wi + 1,
                    other + 1
                );
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Acceptance
// ---------------------------------------------------------------------------

/// The spine alone must reach every world and finish the game. This is the
/// instrument's own calibration as much as a property: the per-world builder
/// already guarantees each world is completable from its own start, and the
/// spine deposits the player on exactly that start — so a failure here means
/// the maze walker or the global fixpoint is wrong, not the map.
///
/// Run with no pads, because a pad can only ADD edges: if the spine-only maze
/// is solvable, every padded one built on it is too.
#[test]
fn the_spine_alone_completes_the_maze() {
    let Some(raw) = load_rom() else { return };
    for seed in 0..census_seeds(8) {
        let (_, result) = census_build(&raw, seed);
        let state = GlobalState::from_build(&result, &IDENTITY_SPINE, 0);
        let s = state.spheres();
        assert!(s.solvable, "seed {seed}: spine-only maze unsolvable\n{}", s.spoiler());
        assert_eq!(
            s.wands_at_goal,
            7,
            "seed {seed}: the spine passes all 7 airships before the castle\n{}",
            s.spoiler()
        );
        // `wands_are_collectable`, at the hardest setting the dial reaches.
        // The spine visits all seven airships on the way, so K = 7 holds; the
        // number only becomes interesting once pads let the player skip ahead.
        let hardest = GlobalState::from_build(&result, &IDENTITY_SPINE, 7);
        assert!(
            hardest.wands_are_collectable(&hardest.spheres()),
            "seed {seed}: K=7 unsatisfiable on a spine-only maze"
        );
    }
}

/// Invariant 3, both readings, measured side by side — and the gap between them
/// is the point.
///
/// The charter asked for an exit reachable with **zero keys**. That is stricter
/// than safety needs: a fortress inside the start region is a key the player
/// can go and get, and what actually soft-locks is a start region with no
/// ungated exit AND no fortress that opens one. The strict form was measured at
/// ~56% on the null model against 100% for the correct one, which is what made
/// the start-region rule a cheap guard rather than a placement phase — and why
/// the 16-pad budget is free for maze shaping.
#[test]
fn start_region_exit_rate() {
    let Some(raw) = load_rom() else { return };
    let seeds = census_seeds(8);
    let mut strict = 0usize;
    let mut escapable = 0usize;
    let mut total = 0usize;
    let mut trapped = Vec::new();
    for seed in 0..seeds {
        let (_, state, _) = generated(&raw, seed, &Knobs::default(), 0);
        for wi in 0..8 {
            total += 1;
            strict += usize::from(state.start_region_has_exit(wi));
            if state.start_region_escapable(wi) {
                escapable += 1;
            } else {
                trapped.push(format!("seed {seed} W{}", wi + 1));
            }
        }
    }
    let pct = |n: usize| 100.0 * n as f64 / total as f64;
    eprintln!(
        "  strict  (ungated exit, the charter's wording): {strict}/{total} = {:.1}%",
        pct(strict)
    );
    eprintln!(
        "  correct (ungated exit OR a fort that opens one): {escapable}/{total} = {:.1}%",
        pct(escapable)
    );
    if !trapped.is_empty() {
        eprintln!("  worlds that could strand a player: {}", trapped.join(", "));
    }
    // The correct form is a safety property, not a preference: a world that
    // fails it can strand a player who arrives with nothing. The generator
    // spends a hub pad on any that do, so this is measuring how often that
    // rescue is even needed — and the answer decides whether the start-region
    // rule is a placement phase or a cheap guard.
    assert!(
        escapable >= strict,
        "the weaker form must hold wherever the stricter one does: {escapable} < {strict}"
    );
}

/// Content the maze holds, as the verifier sees it — a sanity read on the
/// wrapper rather than on the maze: every level, fort and pad the build placed
/// has to appear in the spoiler log's reached set of a solvable seed, or the
/// log is lying about coverage.
#[test]
fn every_placed_slot_is_reached() {
    let Some(raw) = load_rom() else { return };
    for seed in 0..census_seeds(4) {
        let (_, state, _) = generated(&raw, seed, &Knobs::default(), 0);
        let s = state.spheres();
        assert!(s.solvable, "seed {seed}: unsolvable\n{}", s.spoiler());

        let reached: std::collections::HashSet<_> =
            s.spheres.iter().flat_map(|x| x.reached.iter().copied()).collect();
        for w in state.worlds.iter().filter(|w| state.in_maze[w.world_idx]) {
            for slot in w.slots.iter().filter(|s| s.kind != SlotKind::HammerBro) {
                assert!(
                    reached.contains(&(w.world_idx, slot.pos)),
                    "seed {seed} W{} {:?} at {:?} never became reachable\n{}",
                    w.world_idx + 1,
                    slot.kind,
                    slot.pos,
                    s.spoiler()
                );
            }
        }
    }
}

/// The weighted roll is reachable at every count and biased to 1.
#[test]
fn pad_count_roll_is_biased_to_one() {
    let mut rng = ChaCha8Rng::seed_from_u64(9);
    let mut hist = [0usize; 4];
    for _ in 0..40_000 {
        hist[super::graph::roll_count(&mut rng)] += 1;
    }
    assert!(hist.iter().all(|&n| n > 0), "every count 0..=3 must be reachable: {hist:?}");
    let most = hist.iter().enumerate().max_by_key(|&(_, n)| n).map(|(i, _)| i);
    assert_eq!(most, Some(1), "1 pad must be the modal count: {hist:?}");
}

/// What terrain the roles actually have to work with — measured before any role
/// placer existed, because a role whose pool is empty is a design that cannot
/// be built.
///
/// `island_sites` is the pool the charter's headline shape needed: a cell no
/// walk reaches, which only a pad can put a player on. It was measured **empty
/// over 800 world-seeds**, which is why there is no `Island` pad role — see
/// `docs/world_maze_design.md`, "The island pad, and why v1 does not have one".
///
/// **It is no longer empty**, and this census is where that shows: 26 sites in
/// 160 world-seeds at the time of writing, concentrated in a couple of worlds.
/// The placer still never draws from the pool (`WorldTerrain::all_sites` offers
/// hub and gated only), so nothing is broken by it — but the measurement the
/// role was declined on has moved, and whether to build the role is a design
/// call, not a cleanup. Printed rather than asserted for that reason.
#[test]
#[ignore]
fn maze_terrain_pools_census() {
    let Some(raw) = load_rom() else { return };
    let seeds = census_seeds(50);
    let mut hub = 0usize;
    let mut gated = 0usize;
    let mut island = 0usize;
    let mut ungated_target = 0usize;
    let mut worlds_without_hub = 0usize;
    let mut per_world_island = [0usize; 8];

    for seed in 0..seeds {
        let (_, result) = census_build(&raw, seed);
        let state = GlobalState::from_build(&result, &IDENTITY_SPINE, 0);
        for w in &state.worlds {
            let t = super::roles::classify(w, &state.locks, &state.reserved_in(w.world_idx));
            hub += t.hub_sites.len();
            gated += t.gated_sites.len();
            island += t.island_sites.len();
            per_world_island[w.world_idx] += t.island_sites.len();
            ungated_target += t.target_ungated as usize;
            if t.hub_sites.is_empty() {
                worlds_without_hub += 1;
            }
        }
    }

    let n = (seeds * 8) as f64;
    eprintln!("\n=== maze terrain pools, {seeds} seeds ({} world-seeds) ===", seeds * 8);
    eprintln!("  hub sites      mean {:.1} per world", hub as f64 / n);
    eprintln!("  gated sites    mean {:.1}", gated as f64 / n);
    eprintln!(
        "  island sites   {island} total, mean {:.2}, by world {per_world_island:?}",
        island as f64 / n
    );
    eprintln!(
        "  worlds with NO hub site        {worlds_without_hub}/{} = {:.1}%",
        seeds * 8,
        100.0 * worlds_without_hub as f64 / n
    );
    eprintln!(
        "  target ungated (invariant 3 by terrain alone) {ungated_target}/{} = {:.1}%",
        seeds * 8,
        100.0 * ungated_target as f64 / n
    );
}

// ---------------------------------------------------------------------------
// The generator
// ---------------------------------------------------------------------------

/// The generator's two hard guarantees, at every knob setting: the maze is
/// finishable and no world can strand a player who arrives in it.
///
/// This is not a measurement — a failure is a bug, not a number. It runs the
/// knobs at both extremes and the null because the guarantee has to survive the
/// whole dial, not just the default.
#[test]
fn the_generator_never_ships_an_unwinnable_maze() {
    let Some(raw) = load_rom() else { return };
    let arms: [(&str, Knobs); 3] = [
        ("null", Knobs::default()),
        ("local keys", Knobs { fort_distance_bias: -1.0, ..Default::default() }),
        ("far keys, all pads cross", Knobs { fort_distance_bias: 1.0, foreign_landing_bias: 1.0 }),
    ];
    for seed in 0..census_seeds(4) {
        for (name, knobs) in &arms {
            // K at both ends: the wand gate is the only thing that can make a
            // reachable castle unenterable, so the guarantee has to hold with
            // it wide open and with it demanding every wand in the game.
            for k in [0u8, super::DEFAULT_WANDS_REQUIRED, 7] {
                let (_, state, report) = generated(&raw, seed, knobs, k);
                let name = &format!("{name}, K={k}");
                assert!(
                    report.spheres.solvable,
                    "seed {seed} [{name}]: unwinnable\n{}",
                    report.spheres.spoiler()
                );
                assert!(
                    report.unsafe_worlds.is_empty(),
                    "seed {seed} [{name}]: worlds that can strand a player: {:?}",
                    report.unsafe_worlds
                );
                // Every fortress keeps exactly one lock — the charter's
                // map-legibility rule, and what lets a world's lock count tell the
                // player its fort count before the forts are found.
                let forts: usize = state
                    .worlds
                    .iter()
                    .map(|w| w.slots.iter().filter(|s| s.kind == SlotKind::Fortress).count())
                    .sum();
                let keyed: std::collections::HashSet<_> =
                    state.locks.iter().filter_map(|l| l.fort).collect();
                assert_eq!(
                    keyed.len(),
                    forts,
                    "seed {seed} [{name}]: {} forts but {} of them key a lock",
                    forts,
                    keyed.len()
                );
                assert!(
                    state.pad_edges().len() <= super::graph::PAD_BUDGET,
                    "seed {seed} [{name}]: pad budget blown"
                );
                // The run has to be playable, not merely reachable: the lazy
                // player must actually be able to collect K wands and finish.
                let cost = super::metrics::completion_cost(&state);
                assert!(cost.reached, "seed {seed} [{name}]: no completion path");
                assert_eq!(
                    cost.wand_detours, k as usize,
                    "seed {seed} [{name}]: collected {} wands, gate wants {k}",
                    cost.wand_detours
                );
            }
        }
    }
}

/// **Every pad is half of a pair, and no same-world pair is a stroll.**
///
/// A telepad pair is one link the player can walk both ways: two pad tiles,
/// each pointing at the other. This asserts the whole of that shape, over both
/// extremes of the crossing knob:
///
/// * a pad's destination is another pad's **tile** — never a level, a pipe, a
///   toad house or a bare cell. The playtest report that opened this was a pad
///   that landed on a blank, and the deeper rule it exposed is that a landing
///   the player cannot recognise is a landing they cannot use: arriving on a
///   pad means you can always see you arrived, and can always go on;
/// * that pad points **back**, so no half of a pair is one-way;
/// * no pad points at itself, and no two pads claim the same cell;
/// * a same-world link spans at least [`MIN_SPAN`] grid cells. The census
///   before the rule had a same-world hop of span 0 — a pad that teleported to
///   the tile you stood on;
/// * the total is even and inside the arrival-row budget, because a pair costs
///   two of the sixteen rows.
///
/// **Landing on a pad tile cannot loop.** The enter hook replaces
/// `PRG010_CEA7`, and all three branches that reach it sit downstream of an
/// A-button EDGE test (`Controller1Press`/`Controller2Press` for the 2P path,
/// `Pad_Input AND #PAD_A` for the special-tile and attribute-table paths). It
/// fires when the player COMMITS to the tile they stand on, never on arriving
/// at one — which is what makes a pad-to-pad graph possible at all.
#[test]
fn every_pad_is_half_of_a_pair() {
    // Spelled out rather than read off `graph::SAME_WORLD_MIN_SPAN`, and the
    // difference is not cosmetic: the first cut of this test imported the
    // constant, and setting that constant to 0 then left the test PASSING. A
    // check that reads the number it is pinning pins nothing. This is the
    // number the rule IS, so lowering the generator's constant has to fail
    // here rather than quietly redefine the rule.
    const MIN_SPAN: usize = 8;

    let Some(raw) = load_rom() else { return };
    let arms: [(&str, Knobs); 3] = [
        ("null", Knobs::default()),
        ("all pads stay home", Knobs { foreign_landing_bias: 0.0, fort_distance_bias: 0.0 }),
        ("all pads cross", Knobs { foreign_landing_bias: 1.0, fort_distance_bias: 1.0 }),
    ];
    for seed in 0..census_seeds(4) {
        for (name, knobs) in &arms {
            let (_, state, _) = generated(&raw, seed, knobs, 0);
            let pads = state.pad_edges();
            let tiles: Vec<_> = pads.iter().map(|&(from, _)| from).collect();

            assert!(
                pads.len() <= PAD_BUDGET,
                "seed {seed} [{name}]: {} pads exceeds the {PAD_BUDGET} arrival rows",
                pads.len()
            );
            assert_eq!(
                pads.len() % 2,
                0,
                "seed {seed} [{name}]: {} pads is odd, so one of them is unpaired",
                pads.len()
            );

            let mut unique = tiles.clone();
            unique.sort_unstable();
            unique.dedup();
            assert_eq!(
                unique.len(),
                tiles.len(),
                "seed {seed} [{name}]: two pads claim the same cell"
            );

            // Above the per-world cap a world stops being a place and starts
            // being a switchboard.
            let mut per_world = [0usize; 8];
            for &(world, _) in &tiles {
                per_world[world] += 1;
            }
            for (wi, &n) in per_world.iter().enumerate() {
                assert!(
                    n <= super::graph::PADS_PER_WORLD_MAX,
                    "seed {seed} [{name}] W{}: {n} pads, cap is {}",
                    wi + 1,
                    super::graph::PADS_PER_WORLD_MAX
                );
            }

            for &(from, to) in &pads {
                assert_ne!(
                    from,
                    to,
                    "seed {seed} [{name}]: the pad at W{} {:?} teleports to itself",
                    from.0 + 1,
                    from.1
                );
                assert!(
                    tiles.contains(&to),
                    "seed {seed} [{name}]: the pad at W{} {:?} points at W{} {:?}, where no pad \
                     stands — the player arrives somewhere they cannot leave the same way",
                    from.0 + 1,
                    from.1,
                    to.0 + 1,
                    to.1
                );
                let back = pads.iter().find(|&&(f, _)| f == to).map(|&(_, t)| t);
                assert_eq!(
                    back,
                    Some(from),
                    "seed {seed} [{name}]: W{} {:?} -> W{} {:?} is one-way; its partner points \
                     somewhere else",
                    from.0 + 1,
                    from.1,
                    to.0 + 1,
                    to.1
                );
                if from.0 == to.0 {
                    let d = from.1.0.abs_diff(to.1.0) + from.1.1.abs_diff(to.1.1);
                    assert!(
                        d >= MIN_SPAN,
                        "seed {seed} [{name}]: same-world pair W{} {:?} <-> {:?} spans {d}, under \
                         the {MIN_SPAN} that makes a pad worth its arrival id",
                        from.0 + 1,
                        from.1,
                        to.1,
                    );
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// The pad tile
// ---------------------------------------------------------------------------

/// Engine byte tables [`TILE_TELEPAD`] must be **absent** from, as
/// `(name, file offset, length)`.
///
/// Two tables it must be PRESENT in are checked separately below, because
/// membership is what makes the byte work rather than what would break it.
#[rustfmt::skip]
const FORBIDDEN_TILE_TABLES: [(&str, usize, usize); 9] = [
    // PRG012 — the map reload's own vocabulary.
    ("Map_Removable_Tiles",            0x18447, 8),
    ("Map_RemoveTo_Tiles",             0x1844F, 8),
    ("Map_Completable_Tiles",          0x18457, 5),
    ("Map_CompleteByML_Tiles",         0x1845C, 8),
    ("Map_Bottom_Tiles",               0x18464, 9),
    // PRG011 — completion FX and the marching map objects.
    ("Map_ForcePoofTiles",             0x169E5, 5),
    ("Map_MarchXtraForbidTiles",       0x173A9, 4),
    // PRG010 — what the player may walk OVER. A pad sits on the node lattice,
    // never on a path cell between two nodes, so it needs no membership here;
    // `Map_CheckDoMove` validates only the tile being crossed and then moves a
    // hardcoded two tiles, which is why every panel on the map is absent too.
    ("Map_Object_Valid_Left/Right",    0x15258, 18),
    ("Map_Object_Valid_Down/Up",       0x1526A, 18),
];

/// **The pad's byte is the pad's alone**, and this is the check that says so.
///
/// The playtest: 28 spade panels on the map, nine of them pads, so two thirds
/// of everything that looked like a telepad was an N-Spade card game. The fix
/// is a byte no other system claims — and "no other system" has to be checked
/// against the engine's tables, not assumed, because a map tile's entire
/// behaviour is table membership.
#[test]
fn the_pad_tile_is_in_no_registry() {
    use crate::randomize::rom_data::{
        BACKGROUND_TILES, FORTRESS_TILES, LOCK_TILES, TILE_AIRSHIP, TILE_BONUS_GAME, TILE_BOWSER,
        TILE_NODE, TILE_PIPE, TILE_START, TILE_TELEPAD, TILE_TOAD_HOUSE, VALID_BLANK_TILES,
        VALID_HORZ, VALID_VERT, WAND_GATE_TILE, WATER_GAP_TILE,
    };

    // Our own vocabulary first — these need no ROM.
    assert!(!VALID_HORZ.contains(&TILE_TELEPAD), "the walker would treat a pad as a path cell");
    assert!(!VALID_VERT.contains(&TILE_TELEPAD));
    assert!(!VALID_BLANK_TILES.contains(&TILE_TELEPAD), "a slot could be placed on a pad");
    assert!(!BACKGROUND_TILES.contains(&TILE_TELEPAD), "the walker would read a pad as a wall");
    assert!(!FORTRESS_TILES.contains(&TILE_TELEPAD));
    assert!(!LOCK_TILES.contains(&TILE_TELEPAD));
    for (name, tile) in [
        ("bonus game", TILE_BONUS_GAME),
        ("pipe", TILE_PIPE),
        ("toad house", TILE_TOAD_HOUSE),
        ("airship", TILE_AIRSHIP),
        ("bowser", TILE_BOWSER),
        ("start", TILE_START),
        ("wand gate", WAND_GATE_TILE),
        ("water gap", WATER_GAP_TILE),
        ("bfs node placeholder", TILE_NODE),
    ] {
        assert_ne!(TILE_TELEPAD, tile, "the pad byte is also the {name} byte");
    }

    let Some(rom) = load_rom() else { return };
    for (name, off, len) in FORBIDDEN_TILE_TABLES {
        for i in 0..len {
            assert_ne!(rom.read_byte(off + i), TILE_TELEPAD, "the pad byte is in {name}");
        }
    }

    // No world's vanilla grid uses it, so a pad can never be stamped over
    // something that was already there and mean two things at once.
    for world in 0..8 {
        let grid = crate::randomize::rom_data::read_tile_grid(&rom, world);
        for r in 0..grid.rows() {
            for c in 0..grid.cols {
                assert_ne!(grid.get(r, c), TILE_TELEPAD, "W{} uses the pad byte", world + 1);
            }
        }
    }

    // --- and the two memberships the pad NEEDS ---------------------------

    // `Map_EnterSpecialTiles` (11 entries at 0x14DBF) is how the tile reaches
    // `PRG010_CEA7`, which is where `PAD_ENTER` hangs. Without this the hook
    // never fires and stepping on a pad does nothing at all.
    let enter: Vec<u8> = (0..11).map(|i| rom.read_byte(0x14DBF + i)).collect();
    assert!(enter.contains(&TILE_TELEPAD), "the pad byte is not enterable");

    // `Map_Object_Forbid_LandingTiles` (17 entries at 0x17398) keeps a marching
    // Hammer Bro off the tile. `TILE_BONUS_GAME` is in it too; this is the one
    // registry the pad wants to be in.
    let forbid: Vec<u8> = (0..17).map(|i| rom.read_byte(0x17398 + i)).collect();
    assert!(forbid.contains(&TILE_TELEPAD), "a Hammer Bro could march onto a pad");

    // --- and the threshold, which is a rule and not a table --------------

    // `Tile_Attributes_TS0` (0x18410, four bytes indexed by `tile >> 6`) is the
    // level gate: `MO_NormalMoveEnter` will not let the player walk OFF a tile
    // at or above its page's threshold until it is completed. A pad can never
    // be completed, so a pad byte at or above the threshold would sever every
    // path it stood on.
    let threshold = rom.read_byte(0x18410 + (TILE_TELEPAD >> 6) as usize);
    assert!(
        TILE_TELEPAD < threshold,
        "the pad byte {TILE_TELEPAD:#04X} is at or above its page's enterability threshold \
         {threshold:#04X}, so a player who walks onto a pad could never walk off it"
    );
}

/// **Stamping pads writes no CHR**, and the four patterns it points at are the
/// ones it means to point at.
///
/// The sibling of `wand_gate::the_gate_writes_no_chr`, and it exists for the
/// same shipped bug: an earlier cut drew art into map CHR `$80`-`$83` on a
/// three-legged argument that nothing referenced them, and they turned out to
/// be the corners of the map's window boxes, drawn by a routine that *computes*
/// the index. The pad wears those same four patterns — **by pointing at them**,
/// which is free, rather than by overwriting them, which shredded the map.
#[test]
fn stamping_pads_writes_no_chr() {
    use crate::randomize::rom_data::{PRG012_FILE_BASE, TELEPAD_QUADRANTS, TILE_TELEPAD};

    let Some(raw) = load_rom() else { return };
    let (rom, state, _) = generated(&raw, 1, &Knobs::default(), super::DEFAULT_WANDS_REQUIRED);
    let mut after = rom.clone();
    super::writer::stamp_pad_tiles(&mut after, &state);

    const CHR: usize = 0x40010;
    assert_eq!(after.data[CHR..], rom.data[CHR..], "the maze wrote into CHR");

    for (plane, &pattern) in TELEPAD_QUADRANTS.iter().enumerate() {
        let off = PRG012_FILE_BASE + plane * 256 + TILE_TELEPAD as usize;
        assert_eq!(after.read_byte(off), pattern, "quadrant {plane} of the pad tile");
    }
    // The composition is the only metatile row the maze touches: no other
    // tile's quadrants move, so nothing already on the map changes shape.
    for tile in 0..256usize {
        if tile == TILE_TELEPAD as usize {
            continue;
        }
        for plane in 0..4 {
            let off = PRG012_FILE_BASE + plane * 256 + tile;
            assert_eq!(
                after.read_byte(off),
                rom.read_byte(off),
                "the maze repointed quadrant {plane} of tile {tile:#04X}"
            );
        }
    }
}

/// **No pad shares a tile value with a card game.** The regression test for the
/// report.
///
/// "i played and beat world7 entered a spade which was in w2 and it was the
/// spade game" — measured on that ROM, 28 spade panels and only 9 pads. The pad
/// and the card game were the same byte, so the map could not tell the player
/// which was which and two out of three were a disappointment.
///
/// This runs the **whole randomizer** and reads the finished map back, because
/// the two tiles are stamped by different modules — the overworld writer puts
/// down the card games, `writer::stamp_pad_tiles` puts down the pads — and a
/// disagreement between them is exactly the failure being guarded. The pads are
/// located the way the ROM locates them, by decoding `PAD_ENTER`'s own key
/// rows, so a pad the tables cannot find is not a pad this test believes in
/// either.
///
/// The card-game count is asserted non-zero on purpose: with `keep_n_cards`
/// false the roaming N-Spade object is suppressed, but the 19 fixed panels stay
/// on the map, and a run with no card games left would make this test pass
/// vacuously.
#[test]
fn no_pad_shares_a_tile_with_a_card_game() {
    use crate::randomize::rom_data::{self, TILE_BONUS_GAME, TILE_TELEPAD};
    use crate::randomize::world_persist::{PAD_TABLE_OFF, PORTAL_MAX};
    use crate::randomizer::{Options, randomize};

    let Some(raw) = load_rom() else { return };
    let mut total_pads = 0usize;
    let mut total_spades = 0usize;
    for seed in 0..census_seeds(3) {
        let mut rom = raw.clone();
        randomize(
            &mut rom,
            seed,
            &Options { world_maze: true, palettes: false, ..Default::default() },
        );

        // Where the ROM itself thinks the pads are.
        let table = rom_data::FS_PAD_ENTER + PAD_TABLE_OFF;
        let mut pad_cells = Vec::new();
        for id in 0..PORTAL_MAX {
            let world = rom.read_byte(table + id);
            if world == 0xFF {
                continue; // unclaimed arrival row
            }
            // The engine's own encoding: Y = (grid_row + 2) << 4, and the X
            // byte packs the column within its screen high and the screen low.
            let y = rom.read_byte(table + PORTAL_MAX + id);
            let x = rom.read_byte(table + 2 * PORTAL_MAX + id);
            let row = (y >> 4) as usize - 2;
            let col = (x & 0x0F) as usize * 16 + (x >> 4) as usize;
            pad_cells.push((world as usize, row, col));
        }
        assert!(!pad_cells.is_empty(), "seed {seed}: a maze ROM with no telepads at all");

        for &(world, row, col) in &pad_cells {
            let tile = rom.read_byte(rom_data::map_tile_offset(world, row, col));
            assert_eq!(
                tile,
                TILE_TELEPAD,
                "seed {seed}: the pad keyed at W{} ({row},{col}) holds {tile:#04X}",
                world + 1
            );
        }

        // Now the other direction: every telepad tile on the map is a pad the
        // tables know about, and every spade panel is a card game.
        let mut telepads = 0usize;
        let mut spades = 0usize;
        for world in 0..8 {
            let grid = rom_data::read_tile_grid(&rom, world);
            for row in 0..grid.rows() {
                for col in 0..grid.cols {
                    match grid.get(row, col) {
                        TILE_TELEPAD => {
                            assert!(
                                pad_cells.contains(&(world, row, col)),
                                "seed {seed}: W{} ({row},{col}) wears the telepad tile but no \
                                 arrival row keys it — stepping on it would do nothing",
                                world + 1
                            );
                            telepads += 1;
                        }
                        TILE_BONUS_GAME => {
                            assert!(
                                !pad_cells.contains(&(world, row, col)),
                                "seed {seed}: the pad at W{} ({row},{col}) is still a spade \
                                 panel — the player cannot tell it from a card game",
                                world + 1
                            );
                            spades += 1;
                        }
                        _ => {}
                    }
                }
            }
        }
        assert_eq!(
            telepads,
            pad_cells.len(),
            "seed {seed}: {} arrival rows but {telepads} telepad tiles on the map",
            pad_cells.len()
        );
        assert!(spades > 0, "seed {seed}: no card games left, so this test proved nothing");
        total_pads += telepads;
        total_spades += spades;
    }
    eprintln!("  {total_pads} pads and {total_spades} card games over the census, no tile shared");
}

/// The generator census, and the one that answers "how long is this game".
///
/// ```sh
/// CENSUS_SEEDS=100 cargo test --release --lib maze_generator_census \
///     -- --ignored --nocapture
/// ```
#[test]
#[ignore]
fn maze_generator_census() {
    let Some(raw) = load_rom() else { return };
    let seeds = census_seeds(50);
    let arms: [(&str, Knobs); 4] = [
        ("null (bias 0)", Knobs::default()),
        ("local keys (-1)", Knobs { fort_distance_bias: -1.0, ..Default::default() }),
        ("far keys (+1)", Knobs { fort_distance_bias: 1.0, ..Default::default() }),
        ("far keys + all pads cross", Knobs { fort_distance_bias: 1.0, foreign_landing_bias: 1.0 }),
    ];

    eprintln!("\n=== world-maze generator, {seeds} seeds per arm ===");
    eprintln!(
        "{:<28} {:>7} {:>7} {:>7} {:>8} {:>8} {:>7} {:>7}",
        "arm", "spheres", "width", "foreign", "levels", "forts", "pads", "cross"
    );
    for (name, knobs) in &arms {
        let mut spheres = Vec::new();
        let mut widths = Vec::new();
        let mut foreign = Vec::new();
        let mut content = Vec::new();
        let mut forts = Vec::new();
        let mut pads = Vec::new();
        let mut cross = Vec::new();
        let mut unreached = 0usize;
        // The charter asks for the granted-vs-requested role distribution as a
        // census output, because a role is a request and not a contract: some
        // terrain cannot host what was asked for.
        let mut asked = [0usize; 3];
        let mut got = [0usize; 3];
        let mut denied = 0usize;
        // Pads are paired, and every landing is another pad tile, so the
        // only things left to measure are how many pairs there are, how many
        // of them cross, and how far a same-world pair reaches.
        let mut pairs = 0usize;
        let mut crossings = 0usize;
        let mut hops: Vec<usize> = Vec::new();
        let idx = |r: super::roles::PadRole| match r {
            super::roles::PadRole::Hub => 0,
            super::roles::PadRole::Shortcut => 1,
            super::roles::PadRole::Free => 2,
        };

        for seed in 0..seeds {
            let (_, state, report) = generated(&raw, seed, knobs, 0);
            for pad in &report.pads {
                asked[idx(pad.requested)] += 1;
                got[idx(pad.granted)] += 1;
                if pad.requested != pad.granted && pad.requested != super::roles::PadRole::Free {
                    denied += 1;
                }
                let MazeEdge::Pad { from, to } = pad.edge else { unreachable!() };
                assert_eq!(
                    pad.foreign,
                    from.0 != to.0,
                    "PlacedPad::foreign disagrees with its own edge"
                );
            }
            spheres.push(report.spheres.spheres.len());
            widths.extend(report.spheres.spheres.iter().map(|s| s.width));
            foreign.push(report.fill.foreign_locks);
            let cost = super::metrics::completion_cost(&state);
            if !cost.reached {
                unreached += 1;
            }
            content.push(cost.content);
            forts.push(cost.forts);
            let p = state.pad_edges();
            // One entry per PAIR: take the half whose tile sorts first, so a
            // link is counted once rather than from both ends.
            for &(from, to) in p.iter().filter(|(from, to)| from < to) {
                pairs += 1;
                if from.0 != to.0 {
                    crossings += 1;
                } else {
                    hops.push(from.1.0.abs_diff(to.1.0) + from.1.1.abs_diff(to.1.1));
                }
            }
            pads.push(p.len());
            cross.push(p.iter().filter(|((a, _), (b, _))| a != b).count());
        }
        let mean = |v: &[usize]| v.iter().sum::<usize>() as f64 / v.len().max(1) as f64;
        eprintln!(
            "{name:<28} {:>7.2} {:>7.2} {:>7.2} {:>8.1} {:>8.1} {:>7.2} {:>7.2}",
            mean(&spheres),
            mean(&widths),
            mean(&foreign),
            mean(&content),
            mean(&forts),
            mean(&pads),
            mean(&cross),
        );
        eprintln!("    roles asked hub/shortcut/free {asked:?}  granted {got:?}  denied {denied}");
        eprintln!(
            "    pairs: {pairs} ({:.2}/seed), {crossings} span two worlds ({:.0}%), \
             {} stay home",
            pairs as f64 / seeds as f64,
            100.0 * crossings as f64 / pairs.max(1) as f64,
            hops.len(),
        );
        hops.sort_unstable();
        eprintln!(
            "    same-world pairs: span min {:?} median {:?} mean {:.1}",
            hops.first(),
            hops.get(hops.len() / 2),
            hops.iter().sum::<usize>() as f64 / hops.len().max(1) as f64,
        );
        assert_eq!(unreached, 0, "[{name}]: {unreached} seeds never reached the castle");
    }
    eprintln!("  levels = levels+forts a play-through beats; forts = how many of those were keys");
}

/// How many levels the game actually asks of a player, and how many it forces.
/// The gap between the two is the size of the choice the maze offers.
///
/// ```sh
/// CENSUS_SEEDS=25 cargo test --release --lib maze_game_length_census \
///     -- --ignored --nocapture
/// ```
#[test]
#[ignore]
fn maze_game_length_census() {
    let Some(raw) = load_rom() else { return };
    let seeds = census_seeds(20);
    let knobs = Knobs::default();
    let mut played = Vec::new();
    let mut required = Vec::new();
    let mut total_levels = Vec::new();
    let mut detours = Vec::new();

    for seed in 0..seeds {
        // At the SHIPPING wand requirement, not K=0. K=0 is a legal setting but
        // it is the one where a pad chain can finish a seed in a single level,
        // so measuring the game's length there answers a question nobody asked.
        let (_, state, _) = generated(&raw, seed, &knobs, super::DEFAULT_WANDS_REQUIRED);
        let cost = super::metrics::completion_cost(&state);
        played.push(cost.content);
        detours.push(cost.detours);
        required.push(super::metrics::required_levels(&state));
        total_levels.push(
            state
                .worlds
                .iter()
                .map(|w| w.slots.iter().filter(|s| s.kind == SlotKind::Level).count())
                .sum::<usize>(),
        );
    }

    let mean = |v: &[usize]| v.iter().sum::<usize>() as f64 / v.len().max(1) as f64;
    let stat = |v: &[usize]| {
        let mut s = v.to_vec();
        s.sort_unstable();
        (s[0], s[s.len() / 2], s[s.len() - 1])
    };
    let (pl, pm, ph) = stat(&played);
    let (rl, rm, rh) = stat(&required);
    eprintln!(
        "\n=== how long is a maze game, {seeds} seeds, K={} ===",
        super::DEFAULT_WANDS_REQUIRED
    );
    eprintln!("  levels in the game        {:.0}", mean(&total_levels));
    eprintln!(
        "  BEATEN on a completion    mean {:.1}  min {pl}  median {pm}  max {ph}",
        mean(&played)
    );
    eprintln!(
        "  strictly REQUIRED         mean {:.1}  min {rl}  median {rm}  max {rh}",
        mean(&required)
    );
    eprintln!("  fort detours              mean {:.1}", mean(&detours));
    eprintln!(
        "  so a run plays {:.0}% of the game, and {:.0}% of it is unavoidable",
        100.0 * mean(&played) / mean(&total_levels),
        100.0 * mean(&required) / mean(&total_levels),
    );
}

/// **The K sweep** — what the wand gate is worth, and what it costs.
///
/// K is the mode's difficulty dial: the castle will not open until the player
/// holds K of the 7 wands, and a wand is an airship clear. Without it a pad
/// chain can drop the player next to the castle almost immediately — measured,
/// some seeds finish in ONE level. This is the table that says what each K buys.
///
/// ```sh
/// CENSUS_SEEDS=40 cargo test --release --lib maze_wand_gate_sweep \
///     -- --ignored --nocapture
/// ```
#[test]
#[ignore]
fn maze_wand_gate_sweep() {
    let Some(raw) = load_rom() else { return };
    let seeds = census_seeds(30);
    let knobs = Knobs::default();

    eprintln!("\n=== the wand gate, {seeds} seeds per K ===");
    eprintln!(
        "{:>2}  {:>7} {:>7} {:>7} {:>7}  {:>7} {:>7} {:>9}",
        "K", "levels", "min", "median", "max", "unwin", "nopath", "wand trips"
    );
    for k in 0..=7u8 {
        let mut played = Vec::new();
        let mut wand_trips = Vec::new();
        let mut unwinnable = 0usize;
        let mut no_path = 0usize;
        for seed in 0..seeds {
            let (_, state, report) = generated(&raw, seed, &knobs, k);
            if !report.spheres.solvable {
                unwinnable += 1;
                continue;
            }
            let cost = super::metrics::completion_cost(&state);
            if !cost.reached {
                no_path += 1;
                continue;
            }
            played.push(cost.content);
            wand_trips.push(cost.wand_detours);
        }
        played.sort_unstable();
        let mean = |v: &[usize]| v.iter().sum::<usize>() as f64 / v.len().max(1) as f64;
        eprintln!(
            "{k:>2}  {:>7.1} {:>7} {:>7} {:>7}  {:>7} {:>7} {:>9.1}",
            mean(&played),
            played.first().copied().unwrap_or(0),
            played.get(played.len() / 2).copied().unwrap_or(0),
            played.last().copied().unwrap_or(0),
            unwinnable,
            no_path,
            mean(&wand_trips),
        );
    }
    eprintln!("  levels = levels+forts a play-through beats, of 62 levels + 17 forts");
}

/// `the_maze_leaves_every_pipe_alone`, for a **generated** maze rather than a
/// hand-specified one.
///
/// The property the whole telepad choice rests on: a pad is a tile, so it
/// spends no transit room and no destination-table slot, and the four pipe
/// destination tables plus every world pointer table come out byte-identical
/// to what the overworld writer left. The existing test in `world_persist`
/// asserts this for a POC ROM; this asserts it for what the generator actually
/// emits, which is where a stray write would come from.
#[test]
fn a_generated_maze_writes_only_pad_tiles_and_free_space() {
    use crate::randomize::rom_data;

    let Some(raw) = load_rom() else { return };
    for seed in 0..census_seeds(3) {
        let (rom, state, _) =
            generated(&raw, seed, &Knobs::default(), super::DEFAULT_WANDS_REQUIRED);

        let mut after = rom.clone();
        super::writer::stamp_pad_tiles(&mut after, &state);
        crate::randomize::world_persist::apply(&mut after, &super::writer::telepad_specs(&state));

        // Every byte the maze changed inside the map grids must be a pad tile
        // it meant to stamp. Anything else is a bug that a playtest would find
        // as a hole in the map.
        let pads: std::collections::HashSet<_> =
            state.pad_edges().into_iter().map(|(f, _)| f).collect();
        for wi in 0..8 {
            let grid = rom_data::read_tile_grid(&rom, wi);
            for r in 0..grid.rows() {
                for c in 0..grid.cols {
                    let off = rom_data::map_tile_offset(wi, r, c);
                    if rom.read_byte(off) == after.read_byte(off) {
                        continue;
                    }
                    assert!(
                        pads.contains(&(wi, (r, c))),
                        "seed {seed}: the maze changed W{} ({r},{c}) and no pad stands there",
                        wi + 1
                    );
                    assert_eq!(
                        after.read_byte(off),
                        rom_data::TILE_TELEPAD,
                        "seed {seed}: pad at W{} ({r},{c}) is not a telepad tile",
                        wi + 1
                    );
                }
            }
        }
    }
}

/// A short spine — `world_count < 7` — still makes a winnable maze, and the
/// worlds it leaves out are reachable only by pad.
///
/// This is `world_count` keeping its meaning under the mode rather than being
/// forced to 7. The off-spine worlds have no airship edge into them, so the
/// only way in is a telepad; the guarantee that has to survive is that nothing
/// required is stranded behind one.
#[test]
fn a_short_spine_still_finishes() {
    let Some(raw) = load_rom() else { return };
    for seed in 0..census_seeds(4) {
        let (_, result) = census_build(&raw, seed);
        // World order's own shape: a shuffled prefix, Dark Land always last.
        for count in [3usize, 5] {
            let mut pool: Vec<usize> = (0..7).collect();
            let mut rng = ChaCha8Rng::seed_from_u64(seed ^ count as u64);
            pool.shuffle(&mut rng);
            let mut spine: Vec<usize> = pool[..count].to_vec();
            spine.push(7);

            // K cannot exceed the airships the spine actually offers.
            let k = (super::DEFAULT_WANDS_REQUIRED as usize).min(count) as u8;
            let (state, report) = super::generate(&result, &spine, k, &Knobs::default(), &mut rng);
            assert!(
                report.spheres.solvable,
                "seed {seed} spine {spine:?}: unwinnable\n{}",
                report.spheres.spoiler()
            );
            assert!(
                report.unsafe_worlds.is_empty(),
                "seed {seed} spine {spine:?}: worlds that can strand a player: {:?}",
                report.unsafe_worlds
            );
            let cost = super::metrics::completion_cost(&state);
            assert!(cost.reached, "seed {seed} spine {spine:?}: no completion path");
        }
    }
}

/// **The rows the ROM side is handed are the maze's whole assignment**, not the
/// cross-world half of it.
///
/// The failure this guards is silent and expensive, and it shipped: the ROM used
/// to take only `is_foreign()` locks from here and let the *overworld builder's*
/// original pairing stand for the rest. `fill` starts from that pairing and
/// moves by swapping two locks' forts, so every swap that left both locks in
/// their own worlds was discarded on the way to the cartridge — and a fortress
/// whose lock had been swapped away kept emitting the stale local key, so it
/// opened two. Measured before the fix: 33.1% of same-world locks were opened
/// by the wrong fortress, in 59 of 60 seeds.
///
/// So this checks the assignment as a whole: one row per lock, each naming the
/// fortress `fill` actually chose, and the fort/lock bijection intact.
#[test]
fn lock_key_rows_match_the_whole_assignment() {
    let Some(raw) = load_rom() else { return };
    let (mut foreign, mut seeds_with_any) = (0u64, 0u64);
    let seeds = census_seeds(4);
    for seed in 0..seeds {
        let (_, state, _) = generated(&raw, seed, &Knobs::default(), super::DEFAULT_WANDS_REQUIRED);
        let rows = super::writer::lock_keys(&state);

        assert_eq!(
            rows.len(),
            state.locks.iter().filter(|l| l.fort.is_some()).count(),
            "seed {seed}: a lock was dropped on the way to the ROM"
        );

        // The bijection `fill` maintains has to survive the trip: one fortress
        // opens one lock, one lock has one fortress.
        let mut keys: Vec<_> = rows.iter().map(|r| (r.key_world, r.key_pos)).collect();
        let mut targets: Vec<_> = rows.iter().map(|r| (r.target_world, r.target_pos)).collect();
        keys.sort_unstable();
        targets.sort_unstable();
        let (before_k, before_t) = (keys.len(), targets.len());
        keys.dedup();
        targets.dedup();
        assert_eq!(keys.len(), before_k, "seed {seed}: a fortress opens two locks");
        assert_eq!(targets.len(), before_t, "seed {seed}: a lock has two fortresses");

        foreign += rows.iter().filter(|r| r.key_world != r.target_world).count() as u64;
        seeds_with_any += u64::from(rows.iter().any(|r| r.key_world != r.target_world));
        for row in &rows {
            // The named cell really holds that fortress, and the lock really
            // sits where the row says.
            let fort = state.worlds[row.key_world]
                .slots
                .iter()
                .find(|s| s.pos == row.key_pos)
                .expect("fort row names a cell with no slot");
            assert_eq!(fort.kind, SlotKind::Fortress, "seed {seed}: fort row names a non-fortress");
            let lock = state
                .locks
                .iter()
                .find(|l| l.world == row.target_world && l.pos == row.target_pos)
                .expect("lock row names a cell with no lock");
            assert_eq!(
                lock.fort.map(|f| (f.world, f.section)),
                Some((row.key_world, fort.section)),
                "seed {seed}: the row pairs a lock with a fortress that does not open it"
            );
        }
    }
    // **The cross-world checks pass vacuously at zero foreign rows**, which is
    // how the old version of this test sat green while nobody could confirm the
    // mode's headline feature existed. Measured over 200 seeds when that was
    // added: every seed had at least one, 51.5% of all locks were foreign,
    // median 9 per seed. The floor is set far below that — this guards "the
    // feature is switched on", not the distribution, which
    // `maze_null_model_baselines` owns.
    assert_eq!(
        seeds_with_any, seeds,
        "only {seeds_with_any} of {seeds} seeds have a cross-world lock — the mode's \
         headline shape is not being generated"
    );
    assert!(
        foreign >= seeds * 2,
        "{foreign} cross-world locks over {seeds} seeds is below the floor of 2/seed"
    );
}

/// **The pads must not overflow the packed completion store.**
///
/// `TILE_TELEPAD` (`$DF`) is in neither `Map_Completable_Tiles` nor
/// `Map_Removable_Tiles`, so a pad claims no stencil cell and no plane bit —
/// one of the reasons that byte was chosen over the spade panel it replaced.
/// When a pad WAS a spade panel every one of them grew both planes and the
/// worst measured case sat at 43 of `PLANE_RESERVE`'s 48 bytes; it is 41 now.
/// This runs the knobs that place the most pads, so if the property ever
/// quietly stops holding the count moves here first.
///
/// The failure mode is not a crash: the planes would silently run past their
/// reserve into the arrival variables that live immediately after them, and the
/// symptom would be a corrupted teleport several worlds later. That is why this
/// is an assert and not a census.
#[test]
fn the_pads_still_fit_the_packed_store() {
    use crate::randomize::completion_bits::{CompletionMap, PLANE_RESERVE};

    let Some(raw) = load_rom() else { return };
    let mut worst = 0usize;
    let mut worst_seed = 0;
    for seed in 0..census_seeds(12) {
        // The knobs that place the most pads: every id spent, all crossing.
        let knobs = Knobs { foreign_landing_bias: 1.0, fort_distance_bias: 1.0 };
        let (rom, state, _) = generated(&raw, seed, &knobs, super::DEFAULT_WANDS_REQUIRED);
        let mut after = rom.clone();
        super::writer::stamp_pad_tiles(&mut after, &state);
        let used = CompletionMap::from_rom(&after).mirror_offset();
        if used > worst {
            worst = used;
            worst_seed = seed;
        }
        assert!(
            used <= PLANE_RESERVE,
            "seed {seed}: {} pads pushed the packed plane to {used} bytes, past PLANE_RESERVE {PLANE_RESERVE}",
            state.pad_edges().len()
        );
    }
    eprintln!("  worst packed plane with pads: {worst} of {PLANE_RESERVE} (seed {worst_seed})");
}

/// **The maze holds under start↔airship swap**, including on seeds where every
/// eligible world is swapped.
///
/// SAS moves both anchors the maze is built on: the spine edge leaves the
/// airship tile and lands on the destination's start tile, and both of those
/// move. It has already produced one maze bug — the visited marker assumed
/// every world starts at column 2 of screen 0, which SAS falsifies for up to
/// five worlds in eight, silently dropping them out of the whistle cycle.
///
/// The censuses roll SAS per world at 50/50, so they cover it on average and
/// never at the extreme. This walks up the seeds until it has seen a fully
/// swapped seed and asserts the guarantees on every one along the way — and
/// fails if it never found one, so it cannot quietly stop testing the thing it
/// is named after.
#[test]
fn the_maze_holds_under_start_airship_swap() {
    let Some(raw) = load_rom() else { return };
    let mut swapped_worlds = 0usize;
    let mut total_worlds = 0usize;

    // A fully swapped seed is a 1-in-128 event, so finding one by building
    // mazes until it turns up would cost a hundred builds for one data point.
    // `pick_swaps` needs only a catalog, so scan for the seed cheaply and then
    // pay for that one build.
    let full_seed = (0u64..4096)
        .find(|&seed| {
            let mut catalog = NodeCatalog::build(&raw, false);
            let mut rng = ChaCha8Rng::seed_from_u64(seed);
            start_airship_swap::pick_swaps(&mut catalog, &mut rng);
            catalog.start_airship_swapped.iter().filter(|&&b| b).count() == 7
        })
        .expect("no seed in 4096 swaps all seven worlds — pick_swaps has changed");

    for seed in (0..census_seeds(24)).chain(std::iter::once(full_seed)) {
        let ((_, result), swaps) = census_build_swaps(&raw, seed);
        let n = swaps.iter().filter(|&&b| b).count();
        swapped_worlds += n;
        total_worlds += 7; // pick_swaps covers W1-W7; W8 keeps Bowser's castle

        let mut rng = ChaCha8Rng::seed_from_u64(seed ^ 0x5A5A_5A5A);
        let (state, report) = super::generate(
            &result,
            &IDENTITY_SPINE,
            super::DEFAULT_WANDS_REQUIRED,
            &Knobs::default(),
            &mut rng,
        );
        assert!(
            report.spheres.solvable,
            "seed {seed} ({n} worlds swapped): unwinnable\n{}",
            report.spheres.spoiler()
        );
        assert!(
            report.unsafe_worlds.is_empty(),
            "seed {seed} ({n} swapped): worlds that can strand a player: {:?}",
            report.unsafe_worlds
        );
        let cost = super::metrics::completion_cost(&state);
        assert!(cost.reached, "seed {seed} ({n} swapped): no completion path");

        // W8 is never swapped, so the wand gate's chokepoint argument is
        // untouched — assert it rather than leave it implied.
        assert!(!swaps[7], "seed {seed}: W8 was swapped; the wand gate assumes it is not");
    }

    eprintln!(
        "  start↔airship swap: {swapped_worlds}/{total_worlds} eligible worlds swapped \
         ({:.0}%), plus seed {full_seed} with all seven",
        100.0 * swapped_worlds as f64 / total_worlds as f64
    );
}

/// **Every world has a fortress reachable with every lock closed.**
///
/// This is the fact [`super::fill`] was written against the negation of. Its
/// header justified the swap search by claiming the charter's constructive fill
/// "needs a fortress inside the start region with every lock closed — and the
/// per-world builder deliberately puts forts *off* the forced path, so the
/// start region frequently holds none."
///
/// It cannot hold none. A lock is opened by beating its fortress, and reaching
/// that fortress cannot require opening the lock it opens — so the chain has to
/// bottom out at a fortress reachable with everything shut. Measured when this
/// was written: **480 of 480 worlds over 60 seeds**, never fewer than one, and
/// up to four.
///
/// The builder does put forts off the *forced path*, which is a different
/// property — `forced_fort_metric` measures it and it is working as designed.
/// The doc slid from "off the forced path" to "behind a lock" and a whole
/// algorithm was chosen on the difference.
#[test]
fn every_world_has_a_fortress_in_its_start_region() {
    use crate::randomize::map_walker::walk_reachable;
    use crate::randomize::overworld_build::{SlotKind, from_built, stamp_slots};

    let Some(raw) = load_rom() else { return };
    let seeds = census_seeds(8);
    let mut hist = [0usize; 8];
    for seed in 0..seeds {
        let (_, result) = census_build(&raw, seed);
        for w in &result.worlds {
            let mut g = w.grid.clone();
            stamp_slots(&mut g, &w.slots);
            for lock in &w.locks {
                g.set(lock.pos.0, lock.pos.1, lock.gap_tile);
            }
            let ws = from_built(w);
            let reach = walk_reachable(&g, &w.pipe_pairs, ws.start, w.world_idx);
            let n = w
                .slots
                .iter()
                .filter(|s| s.kind == SlotKind::Fortress)
                .filter(|s| reach.contains(s.pos))
                .count();
            assert!(
                n > 0,
                "seed {seed} W{}: no fortress is reachable with every lock closed, so no lock \
                 could ever be opened",
                w.world_idx + 1
            );
            hist[n.min(7)] += 1;
        }
    }
    eprintln!("  forts reachable with all locks closed: {hist:?} (index = count)");
}

/// **A fortress opens exactly one lock, and a lock has exactly one fortress.**
///
/// The charter's map-legibility rule — a lock breaking is the only feedback that
/// says which fortress did it — and the reason a world's lock count tells the
/// player its fort count. The builder gets it by construction (no two locks in a
/// world share a `fort_section`) and `maze::fill` preserves it (a swap trades
/// two forts rather than handing one out), but neither states it, and the ROM
/// broke it once by splicing the two halves together.
#[test]
fn fort_and_lock_are_one_to_one() {
    use crate::randomize::overworld_build::SlotKind;

    let Some(raw) = load_rom() else { return };
    for seed in 0..census_seeds(8) {
        let (_, result) = census_build(&raw, seed);
        for w in &result.worlds {
            let forts = w.slots.iter().filter(|s| s.kind == SlotKind::Fortress).count();
            assert_eq!(
                w.locks.len(),
                forts,
                "seed {seed} W{}: {} locks against {forts} fortresses",
                w.world_idx + 1,
                w.locks.len()
            );
            let mut sections: Vec<usize> = w.locks.iter().map(|l| l.fort_section).collect();
            sections.sort_unstable();
            let before = sections.len();
            sections.dedup();
            assert_eq!(
                sections.len(),
                before,
                "seed {seed} W{}: two locks share a fortress",
                w.world_idx + 1
            );
        }
    }
}

/// **The charter's constructive fill does not stall — the swap search works
/// around a problem that is not there.**
///
/// [`super::fill`] chose a swap search because a forward fill "can stall, and on
/// this map it stalls often". Measured, with the frontier taken in arbitrary
/// order it stalls on 28% of seeds; ordering the frontier by how much territory
/// opening a gate reveals takes that to **zero**, and improves everything else
/// at the same time:
///
/// | | first-frontier | territory-ordered |
/// |---|---|---|
/// | stalled | 17/60 | **0/60** |
/// | mean keys to choose from | 4.17 | **6.04** |
/// | a cross-world key was available | 74% | **87%** |
///
/// The second column is the mode's own formula — a lock whose key is in another
/// world, so the player must follow a telepad — available at 87% of steps and
/// aimed at by nothing today: `Knobs::fort_distance_bias` defaults to `0.0`,
/// which the code itself calls a random walk.
///
/// ```sh
/// CENSUS_SEEDS=200 cargo test --release --lib forward_fill -- --ignored --nocapture
/// ```
#[test]
#[ignore]
fn forward_fill_terminates_when_ordered_by_territory() {
    use super::super::rom_data::Pos;
    use super::walk::walk_maze;
    use super::{FortRef, GlobalState};
    use std::collections::HashSet;

    let Some(raw) = load_rom() else { return };
    let seeds = census_seeds(60);
    // Never beaten, so an unassigned lock stays shut.
    let placeholder = FortRef { world: 0, section: 255 };

    for smart in [false, true] {
        let (mut ok, mut stalled) = (0usize, 0usize);
        let (mut steps, mut choice_sum, mut had_cross, mut chose_cross) =
            (0usize, 0usize, 0usize, 0usize);

        for seed in 0..seeds {
            let (_, result) = census_build(&raw, seed);
            let mut rng = ChaCha8Rng::seed_from_u64(seed ^ 0x5EED_1234);
            let mut state =
                GlobalState::from_build(&result, &IDENTITY_SPINE, super::DEFAULT_WANDS_REQUIRED);
            let pads = super::graph::plan_pads(&state, &Knobs::default(), &mut rng);
            state.add_pads(pads.iter().map(|p| p.edge).collect());

            for l in state.locks.iter_mut() {
                l.fort = Some(placeholder);
            }
            let forts: Vec<(FortRef, Pos)> = state
                .worlds
                .iter()
                .flat_map(|w| {
                    w.slots
                        .iter()
                        .filter(|s| s.kind == SlotKind::Fortress)
                        .map(|s| (FortRef { world: w.world_idx, section: s.section }, s.pos))
                })
                .collect();

            let links = state.links();
            let mut open: HashSet<FortRef> = HashSet::new();
            let mut used: HashSet<FortRef> = HashSet::new();
            let mut assigned = vec![false; state.locks.len()];
            let stall;

            loop {
                let bases = state.base_grids(&HashSet::new());
                let grids = state.locked_grids(&bases, &open);
                let reach = walk_maze(&state.view(&grids), &links, state.start);

                for (f, pos) in &forts {
                    if !open.contains(f) && reach.contains((f.world, *pos)) {
                        open.insert(*f);
                    }
                }
                let available: Vec<FortRef> = forts
                    .iter()
                    .map(|(f, _)| *f)
                    .filter(|f| open.contains(f) && !used.contains(f))
                    .collect();

                let frontier: Vec<usize> = (0..state.locks.len())
                    .filter(|&i| !assigned[i])
                    .filter(|&i| {
                        let l = &state.locks[i];
                        let (r, c) = l.pos;
                        let g = &grids[l.world];
                        let mut n: Vec<(usize, usize)> = vec![];
                        if r > 0 {
                            n.push((r - 1, c));
                        }
                        if c > 0 {
                            n.push((r, c - 1));
                        }
                        if r + 1 < g.rows() {
                            n.push((r + 1, c));
                        }
                        if c + 1 < g.cols {
                            n.push((r, c + 1));
                        }
                        n.into_iter().any(|p| reach.contains((l.world, p)))
                    })
                    .collect();

                if frontier.is_empty() {
                    stall = !assigned.iter().all(|&a| a);
                    break;
                }
                if available.is_empty() {
                    stall = true;
                    break;
                }

                // Which frontier gate to open next.
                let li = if smart {
                    let base: usize = (0..state.worlds.len()).map(|w| reach.world_len(w)).sum();
                    let probe = *open.iter().next().expect("a fort is always beatable first");
                    let mut best = (usize::MIN, frontier[0]);
                    for &i in &frontier {
                        let saved = state.locks[i].fort;
                        state.locks[i].fort = Some(probe);
                        let g2 = state.locked_grids(&bases, &open);
                        let r2 = walk_maze(&state.view(&g2), &links, state.start);
                        state.locks[i].fort = saved;
                        let gain: usize =
                            (0..state.worlds.len()).map(|w| r2.world_len(w)).sum::<usize>() - base;
                        if gain > best.0 {
                            best = (gain, i);
                        }
                    }
                    best.1
                } else {
                    frontier[0]
                };

                let lw = state.locks[li].world;
                let cross: Vec<FortRef> =
                    available.iter().copied().filter(|f| f.world != lw).collect();
                steps += 1;
                choice_sum += available.len();
                had_cross += usize::from(!cross.is_empty());
                let pick = if cross.is_empty() { available[0] } else { cross[0] };
                chose_cross += usize::from(pick.world != lw);

                state.locks[li].fort = Some(pick);
                used.insert(pick);
                assigned[li] = true;
            }
            if stall {
                stalled += 1;
            } else {
                ok += 1;
            }
        }

        let policy = if smart { "territory-ordered" } else { "first-frontier" };
        println!("\n{policy} forward fill over {seeds} seeds:");
        println!(
            "   completed  {ok}   STALLED {stalled}  ({:.0}% stall)",
            100.0 * stalled as f64 / seeds as f64
        );
        println!(
            "   {steps} steps, mean {:.2} keys to choose from; cross-world available {:.0}%, taken {:.0}%",
            choice_sum as f64 / steps as f64,
            100.0 * had_cross as f64 / steps as f64,
            100.0 * chose_cross as f64 / steps as f64
        );
    }
}

/// **A lock the per-world builder calls sealable is sealable for the whole
/// maze — and how many locks are sealable at all depends on K.**
///
/// `secret_exit_safe` is computed by the per-world builder as "this world stays
/// completable with that lock sealed forever", *before any telepad exists*. The
/// worry was that a pad could land behind such a lock and make it load-bearing
/// after all. It cannot: pads are emitted as two directed halves, so a pad
/// behind a sealed lock still works as an *entrance*, and extra connectivity
/// only ever adds reachability. Measured: **0 over-promises at every K**.
///
/// What does move is the size of the sealable pool, because the counterfactual
/// asks "castle reachable AND at least K airship docks reachable". A stranded
/// *fortress* is fine — the player chose not to open that lock and can go back —
/// but a stranded *airship* is only affordable while wands are spare:
///
/// | K | globally sealable, of 680 |
/// |---|---|
/// | 0 | 554 (81%) |
/// | 3 (default) | 551 (81%) |
/// | 7 | **389 (57%)** |
///
/// So 1-F's lock needs checking at the shipping K, not at any K — and nothing
/// asks today.
#[test]
#[ignore]
fn per_world_sealable_locks_are_sealable_for_the_maze() {
    use super::GlobalState;

    let Some(raw) = load_rom() else { return };
    let seeds = census_seeds(40);
    for k in [0u8, 3, 7] {
        let (mut locks, mut per_world, mut global, mut both, mut lost) = (0usize, 0, 0, 0, 0);

        for seed in 0..seeds {
            let (_, result) = census_build(&raw, seed);
            let mut rng = ChaCha8Rng::seed_from_u64(seed ^ 0x5EED_1234);
            let mut state = GlobalState::from_build(&result, &IDENTITY_SPINE, k);
            // Pads first, exactly as `generate` does.
            let pads = super::graph::plan_pads(&state, &Knobs::default(), &mut rng);
            state.add_pads(pads.iter().map(|p| p.edge).collect());

            for (li, l) in state.locks.iter().enumerate() {
                let pw = result.worlds[l.world]
                    .locks
                    .iter()
                    .find(|b| b.pos == l.pos)
                    .is_some_and(|b| b.secret_exit_safe);
                let gl = state.winnable_with_lock_sealed(li);
                locks += 1;
                per_world += usize::from(pw);
                global += usize::from(gl);
                both += usize::from(pw && gl);
                lost += usize::from(pw && !gl);
            }
        }
        println!("\n{locks} locks over {seeds} seeds, pads placed:");
        println!("   per-world secret_exit_safe   {per_world}");
        println!("   globally sealed-safe         {global}");
        println!("   both                         {both}");
        println!("   per-world says safe, maze says NOT: {lost}  <-- the over-promise");
    }
}
