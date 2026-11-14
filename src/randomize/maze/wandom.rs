//! World-maze **Wandom**: the castle's wand count is rolled per seed and kept
//! hidden until the player beats one marked level.
//!
//! # What the player sees
//!
//! The map's wand readout ([`crate::randomize::maze::wand_readout`]) reads
//! `N/×` instead of `N/K`. Somewhere early in the maze one level is drawn as a
//! **pyramid** (`$69`, the vanilla 2-Pyramid tile) instead of a numbered panel.
//! Beating it plays the 1-up jingle and the `×` becomes K for the rest of the
//! game. The oracle king says which world it is in.
//!
//! # The three decisions, all made here
//!
//! - **K** ([`roll_wands`]): uniform over the player's range (default 2-6, [`WANDOM_K`]), capped by the
//!   airships the spine offers, from its own substream so nothing else in the seed moves with it.
//! - **Which level** ([`pick_pyramid`]): an *optional* level — the maze is still
//!   winnable with its cell walled off — in a world near the start, by telepad
//!   distance. See its docs for the order.
//! - **The ROM side** ([`trigger`]): one row in the on-clear table
//!   ([`on_clear`](crate::randomize::maze::on_clear)) that notices the clear.
//!
//! # How the clear is noticed
//!
//! Not in the level. A level placed twice by Deja Vu, a chest ending, a secret
//! exit — anything keyed on the level would have to reason about all of them.
//! Instead it is keyed on **the map cell**: every return to the map alive passes
//! map operation 8 with the player standing on what they just cleared, and
//! [`on_clear`](crate::randomize::maze::on_clear) scans a table of cells after every such return. The pyramid's
//! row stores K's digit tile in [`WANDOM_REVEAL`] and queues the 1-up sound.
//!
//! Beaten, the pyramid poofs and becomes an M/L panel like any level (`$69` is
//! inside page 1's completable window), so it cannot be cleared twice.

use std::collections::{HashSet, VecDeque};

use rand::Rng;
use rand::seq::SliceRandom;

use crate::randomize::maze::maze_state::WANDOM_REVEAL;
use crate::randomize::maze::on_clear::Trigger;
use crate::randomize::maze::wand_gate::MAX_WANDS;
use crate::randomize::maze::{GlobalState, MazeEdge, MazePos};
use crate::randomize::overworld::build::{BuildResult, SlotKind};
use crate::randomize::rom_data::STATUS_GLYPH_DIGIT0;

/// The pyramid: vanilla's 2-Pyramid map tile. It sits in page 1's completable
/// window (`$67`-`$69`), so the level-clear effect poofs it to an M/L panel
/// exactly as it does a numbered panel, and a marching Hammer Bro treats it as
/// an enterable node and hops over it (`Map_MarchValidateTravel`'s
/// `Tile_AttrTable` test).
pub(crate) const TILE_PYRAMID: u8 = 0x69;

/// Salt for [`roll_wands`]' substream: "WANDOM_1".
pub(crate) const WANDOM_SALT: u64 = 0x5741_4E44_4F4D_5F31;

/// The range K is rolled from. The user's call after the first build (which
/// rolled 1-7): the ends are left out, so a hidden K is never "none to speak
/// of" or "every airship".
pub(crate) const WANDOM_K: std::ops::RangeInclusive<u8> = 2..=6;

/// Roll K: uniform over the player's range (default [`WANDOM_K`]), capped at
/// the airships the spine offers. With `world_count < 7` the spine is shorter
/// and fewer wands exist at all, which is the same cap `maze_model` puts on a
/// fixed K; a spine too short for the range's floor rolls whatever it has. A
/// range given backwards is read the other way round.
///
/// The default range draws exactly what the fixed 2-6 did, so a Wandom seed
/// that never touched the range is unchanged.
pub(crate) fn roll_wands<R: Rng>(spine_len: usize, min: u8, max: u8, rng: &mut R) -> u8 {
    let (min, max) = (min.min(max), min.max(max));
    let airships = spine_len.saturating_sub(1).min(usize::from(MAX_WANDS)) as u8;
    let hi = max.min(airships);
    let lo = min.min(hi);
    rng.random_range(lo..=hi)
}

/// The telepad depths searched, in order: 3, then 4, 2, 5, 1, 6, 7, 8.
///
/// Depth 1 is the start world. Three is "a few worlds in": far enough that the
/// pyramid is a detour the player chooses, close enough to matter while they
/// are still deciding where to go. The order widens out from 3, preferring
/// deeper on a tie.
const DEPTH_ORDER: [u32; 8] = [3, 4, 2, 5, 1, 6, 7, 8];

/// Each world's telepad depth from the start world: 1 there, 2 one link away,
/// `None` where no link ever leads.
///
/// A link is a pad (each is one-way; a two-way pair is two pads) or the spine's
/// airship hop. Worlds are nodes and nothing inside a map is modelled, so a
/// world behind a lock still counts as one hop away — this is "how far through
/// the maze is it", not "how soon can you get there".
fn world_depths(state: &GlobalState) -> [Option<u32>; 8] {
    let mut depth = [None; 8];
    let start = state.start.0;
    depth[start] = Some(1);
    let mut queue = VecDeque::from([start]);
    while let Some(w) = queue.pop_front() {
        let d = depth[w].expect("queued worlds have a depth");
        for edge in &state.edges {
            let (from, to) = match *edge {
                MazeEdge::Pad { from, to } => (from.0, to.0),
                MazeEdge::Airship { from_world, to_world } => (from_world, to_world),
            };
            if from == w && depth[to].is_none() {
                depth[to] = Some(d + 1);
                queue.push_back(to);
            }
        }
    }
    depth
}

/// Pick the pyramid's cell, or `None` when no world holds an eligible level.
///
/// Searched by [`DEPTH_ORDER`]: every eligible level in the depth-3 worlds is
/// shuffled and tried in turn, and only when none qualifies does the search
/// move on to depth 4, and so on. A level qualifies when:
///
/// - it is a plain level slot — not a hand trap or troll pipe, which have tiles
///   of their own;
/// - the player can reach it at all (it is in some sphere of the full maze);
/// - **it is optional**: the maze is still winnable with its cell walled off.
///   That is the counterfactual the required-level count asks, so a pyramid
///   can never be the only way to a lock, a key or the castle.
///
/// `taken` holds obelisks already placed — the hidden-worlds obelisk passes
/// Wandom's. A taken cell is never picked, and it stays walled off while the
/// next one is tested, so every obelisk is optional *together* and not merely
/// one at a time.
///
/// Shuffle-then-first is a uniform pick among a tier's qualifying levels that
/// usually stops after one or two solvability checks rather than running one
/// per level.
pub(crate) fn pick_pyramid<R: Rng>(
    state: &GlobalState,
    rng: &mut R,
    taken: &[MazePos],
) -> Option<MazePos> {
    let depths = world_depths(state);
    let reached: HashSet<MazePos> =
        state.spheres().spheres.iter().flat_map(|s| s.reached.iter().copied()).collect();

    for target in DEPTH_ORDER {
        let mut tier: Vec<MazePos> = state
            .worlds
            .iter()
            .filter(|w| depths[w.world_idx] == Some(target))
            .flat_map(|w| {
                w.slots
                    .iter()
                    .filter(|s| s.kind == SlotKind::Level && !s.is_hand_trap && !s.is_troll_pipe)
                    .map(move |s| (w.world_idx, s.pos))
            })
            .collect();
        tier.shuffle(rng);
        for cell in tier {
            if !reached.contains(&cell) || taken.contains(&cell) {
                continue;
            }
            let blocked: HashSet<MazePos> = taken.iter().copied().chain([cell]).collect();
            if state.spheres_with_blocked(&blocked).solvable {
                return Some(cell);
            }
        }
    }
    None
}

/// Tell the writer which slot gets the pyramid tile.
pub(crate) fn mark_pyramid(build: &mut BuildResult, cell: MazePos) {
    let (world, pos) = cell;
    let slot = build.worlds[world]
        .slots
        .iter_mut()
        .find(|s| s.pos == pos && s.kind == SlotKind::Level)
        .expect("the pyramid was picked from a level slot the build still has");
    slot.is_pyramid = true;
}

/// The pyramid's on-clear row: beating its cell puts K's digit tile in
/// [`WANDOM_REVEAL`], which the readout draws from. The byte starts at zero, so
/// the dispatcher's `ORA` leaves exactly the digit there.
pub(crate) fn trigger(pyramid: MazePos, k: u8) -> Trigger {
    debug_assert!(k <= MAX_WANDS, "Wandom's K is 0-7");
    Trigger { cell: pyramid, flag: WANDOM_REVEAL, value: STATUS_GLYPH_DIGIT0 + k }
}

/// Point the pyramid's row at another cell, keeping its K: `testrom
/// --obelisk-at`, so the reveal can be playtested on a level next to the start
/// instead of a few worlds in. The obelisk tile stays where the seed drew it.
///
/// `false`, and nothing written, when the ROM has no pyramid row to move (not
/// a Wandom seed).
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn retarget(rom: &mut crate::rom::Rom, cell: MazePos) -> bool {
    crate::randomize::maze::on_clear::retarget(rom, WANDOM_REVEAL, cell)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::randomize::maze::on_clear::{self, installed};
    use crate::rom::Rom;

    /// The row carries K's digit tile into the reveal byte.
    #[test]
    fn the_row_reveals_k_as_a_digit() {
        for k in 1..=MAX_WANDS {
            let t = trigger((2, (4, 20)), k);
            assert_eq!((t.flag, t.value), (WANDOM_REVEAL, STATUS_GLYPH_DIGIT0 + k));
        }
    }

    fn vanilla() -> Option<Rom> {
        let bytes = std::fs::read("roms/Super Mario Bros. 3 (USA) (Rev 1).nes").ok()?;
        Rom::from_bytes(&bytes).ok()
    }

    fn wandom_options() -> crate::Options {
        crate::Options {
            world_maze: true,
            maze_wands: crate::MAZE_WANDS_WANDOM,
            palettes: false,
            ..Default::default()
        }
    }

    /// The pyramid's row in a finished ROM's trigger table.
    fn pyramid_row(rom: &Rom) -> Option<Trigger> {
        installed(rom)?.into_iter().find(|t| t.flag == WANDOM_REVEAL)
    }

    /// **The whole feature on finished ROMs.** For each seed: op 8's no-orb
    /// exit goes to the on-clear trampoline; the pyramid's row is keyed on a
    /// cell that really carries the pyramid tile; the digit it reveals is the K
    /// the maze was built for; and the level is reachable and optional in the
    /// maze that shipped.
    #[test]
    fn wandom_hides_k_behind_a_real_optional_pyramid() {
        use crate::randomize::maze::LAST_MAZE;
        use crate::randomize::overworld::lock_keys::ON_CLEAR_CPU;
        use crate::randomize::rom_data::{FS_FORTRESS_FX, map_tile_offset};
        let Some(base) = vanilla() else {
            eprintln!("SKIP: requires the ROM");
            return;
        };
        let [oc_lo, oc_hi] = ON_CLEAR_CPU.to_le_bytes();
        for seed in 1..=4u64 {
            let mut rom = base.clone();
            crate::pipeline::randomize(&mut rom, seed, &wandom_options());
            let state = LAST_MAZE.with(|s| s.borrow().clone()).expect("the maze ran");

            assert_eq!(
                rom.read_range(FS_FORTRESS_FX + 32, 3),
                &[0x4C, oc_lo, oc_hi],
                "seed {seed}: op 8's no-orb exit is not hooked"
            );
            let row = pyramid_row(&rom).expect("a Wandom seed has a pyramid row");
            let (world, (r, c)) = row.cell;
            assert_eq!(
                rom.read_byte(map_tile_offset(world, r, c)),
                TILE_PYRAMID,
                "seed {seed}: the pyramid row names {:?}, which is not a pyramid",
                row.cell
            );
            let k = row.value - STATUS_GLYPH_DIGIT0;
            assert_eq!(k, state.wands_required, "seed {seed}: the reveal is not the gate's K");
            assert!((1..=MAX_WANDS).contains(&k));
            assert!(
                state.spheres().spheres.iter().any(|s| s.reached.contains(&row.cell)),
                "seed {seed}: the pyramid at {:?} cannot be reached",
                row.cell
            );
            assert!(
                state.spheres_with_blocked(&HashSet::from([row.cell])).solvable,
                "seed {seed}: the pyramid at {:?} is required",
                row.cell
            );
        }
    }

    /// A fixed-K maze seed has no triggers, so op 8's prologue is exactly as it
    /// was and no dispatcher is written.
    #[test]
    fn a_fixed_k_maze_is_not_hooked() {
        use crate::randomize::overworld::lock_keys::ADVANCE_CPU;
        use crate::randomize::rom_data::FS_FORTRESS_FX;
        let Some(mut rom) = vanilla() else {
            eprintln!("SKIP: requires the ROM");
            return;
        };
        let options = crate::Options { maze_wands: 3, ..wandom_options() };
        crate::pipeline::randomize(&mut rom, 1, &options);
        let [lo, hi] = ADVANCE_CPU.to_le_bytes();
        assert_eq!(rom.read_range(FS_FORTRESS_FX + 12, 3), &[0x4C, lo, hi]);
        assert_eq!(installed(&rom), None);
    }

    /// `testrom --obelisk-at` moves the trigger and keeps K; on a ROM with no
    /// dispatcher it writes nothing.
    #[test]
    fn retarget_moves_the_cell_and_keeps_k() {
        let Some(mut rom) = vanilla() else {
            eprintln!("SKIP: requires the ROM");
            return;
        };
        let before = rom.clone();
        assert!(!retarget(&mut rom, (0, (2, 4))), "vanilla has no row to move");
        assert_eq!(rom.read_range(0, 0x60010), before.read_range(0, 0x60010));

        on_clear::apply(&mut rom, &[trigger((3, (4, 20)), 5)]);
        assert!(retarget(&mut rom, (0, (2, 4))));
        assert_eq!(pyramid_row(&rom), Some(trigger((0, (2, 4)), 5)));
    }
    /// **Instrument, not a gate**: where the pyramid lands, how often no level
    /// qualifies, what K comes out, and what the pick costs.
    ///
    /// ```sh
    /// WANDOM_SEEDS=200 cargo test --release --lib wandom_pyramid_census -- --ignored --nocapture
    /// ```
    #[test]
    #[ignore]
    fn wandom_pyramid_census() {
        use crate::randomize::maze::LAST_MAZE;
        use rand::SeedableRng;
        let Some(base) = vanilla() else {
            eprintln!("SKIP: requires the ROM");
            return;
        };
        let n: u64 = std::env::var("WANDOM_SEEDS").ok().and_then(|s| s.parse().ok()).unwrap_or(50);
        let mut by_depth = [0usize; 9];
        let mut ks = [0usize; 8];
        let mut none = 0usize;
        let mut pick_time = std::time::Duration::ZERO;
        for seed in 1..=n {
            let mut rom = base.clone();
            crate::pipeline::randomize(&mut rom, seed, &wandom_options());
            let state = LAST_MAZE.with(|s| s.borrow().clone()).expect("the maze ran");
            ks[usize::from(state.wands_required)] += 1;
            // Re-run the pick on the shipped maze to time it in isolation.
            let mut rng = rand_chacha::ChaCha8Rng::seed_from_u64(seed);
            let t = std::time::Instant::now();
            let cell = pick_pyramid(&state, &mut rng, &[]);
            pick_time += t.elapsed();
            match cell {
                Some((w, _)) => {
                    let d = world_depths(&state)[w].unwrap_or(0) as usize;
                    by_depth[d.min(8)] += 1;
                }
                None => none += 1,
            }
        }
        println!("\nWandom census, {n} seeds");
        println!("pyramid depth (1 = start world):");
        for (d, c) in by_depth.iter().enumerate().skip(1) {
            println!("  depth {d}: {c}");
        }
        println!("  none found: {none}");
        println!("K: {:?} (index = K)", &ks[1..]);
        println!("pick cost: {:.2} ms/seed", pick_time.as_secs_f64() * 1000.0 / n as f64);
    }

    /// A player's range is honoured end to end, 0 and 7 included; a backwards
    /// range is read the other way round; equal ends are a fixed K.
    #[test]
    fn a_custom_range_is_honoured() {
        use rand::SeedableRng;
        let mut rng = rand_chacha::ChaCha8Rng::seed_from_u64(7);
        for (min, max) in [(0, 7), (0, 1), (5, 7), (7, 3), (4, 4)] {
            let (lo, hi) = (min.min(max), min.max(max));
            let mut seen = [false; 8];
            for _ in 0..400 {
                let k = roll_wands(8, min, max, &mut rng);
                assert!((lo..=hi).contains(&k), "{min}-{max} rolled {k}");
                seen[usize::from(k)] = true;
            }
            assert!((lo..=hi).all(|k| seen[usize::from(k)]), "{min}-{max} missed a value");
        }
    }

    /// The default range draws exactly what the old fixed 2-6 did.
    #[test]
    fn the_default_range_matches_the_old_roll() {
        use rand::{Rng, SeedableRng};
        for seed in 0..200 {
            let mut a = rand_chacha::ChaCha8Rng::seed_from_u64(seed);
            let mut b = rand_chacha::ChaCha8Rng::seed_from_u64(seed);
            let old: u8 = a.random_range(2..=6);
            assert_eq!(roll_wands(8, 2, 6, &mut b), old, "seed {seed}");
        }
    }

    #[test]
    fn rolls_stay_in_range_and_cover_it() {
        use rand::SeedableRng;
        let mut rng = rand_chacha::ChaCha8Rng::seed_from_u64(1);
        let mut seen = [false; 8];
        for _ in 0..500 {
            let k = roll_wands(8, 2, 6, &mut rng);
            assert!(WANDOM_K.contains(&k), "K {k} is outside {WANDOM_K:?}");
            seen[usize::from(k)] = true;
        }
        assert!(
            WANDOM_K.clone().all(|k| seen[usize::from(k)]),
            "every K in {WANDOM_K:?} should come up"
        );
        // A two-world spine has one airship, so one wand at most; three worlds
        // have two, which caps the range at its floor.
        assert_eq!(roll_wands(2, 2, 6, &mut rng), 1);
        assert_eq!(roll_wands(3, 2, 6, &mut rng), 2);
        // Four worlds: three airships, so 2 or 3.
        assert!((0..50).all(|_| (2..=3).contains(&roll_wands(4, 2, 6, &mut rng))));
    }
}
