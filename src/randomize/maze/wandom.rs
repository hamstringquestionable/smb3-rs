//! World-maze **Wandom**: the castle's wand count is rolled per seed and kept
//! hidden until the player beats one marked level.
//!
//! # What the player sees
//!
//! The map's wand readout ([`crate::randomize::maze::wand_readout`]) reads
//! `N/?` instead of `N/K`. Somewhere early in the maze one level is drawn as a
//! **pyramid** (`$69`, the vanilla 2-Pyramid tile) instead of a numbered panel.
//! Beating it plays the 1-up jingle and the `?` becomes K for the rest of the
//! game. The oracle king says which world it is in.
//!
//! # The three decisions, all made here
//!
//! - **K** ([`roll_wands`]): uniform over 1-7, capped by the airships the spine
//!   offers, from its own substream so nothing else in the seed moves with it.
//! - **Which level** ([`pick_pyramid`]): an *optional* level — the maze is still
//!   winnable with its cell walled off — in a world near the start, by telepad
//!   distance. See its docs for the order.
//! - **The ROM side** ([`apply`]): the on-clear routine that notices the clear.
//!
//! # How the clear is noticed
//!
//! Not in the level. A level placed twice by Deja Vu, a chest ending, a secret
//! exit — anything keyed on the level would have to reason about all of them.
//! Instead it is keyed on **the map cell**: every return to the map alive passes
//! map operation 8 with the player standing on what they just cleared, and
//! [`crate::randomize::overworld::lock_keys::install_on_clear`] sends op 8's
//! no-orb exit through [`ON_CLEAR_CPU`](crate::randomize::overworld::lock_keys::ON_CLEAR_CPU) with that cell already built into
//! `T1`/`T2`. The routine here compares them against the pyramid's
//! [`cell_key`], and on a match stores K's digit tile in [`WANDOM_REVEAL`] and
//! queues the 1-up sound.
//!
//! Beaten, the pyramid poofs and becomes an M/L panel like any level (`$69` is
//! inside page 1's completable window), so it cannot be cleared twice.

use std::collections::{HashSet, VecDeque};

use rand::Rng;
use rand::seq::SliceRandom;

use crate::randomize::maze::maze_state::WANDOM_REVEAL;
use crate::randomize::maze::wand_gate::MAX_WANDS;
use crate::randomize::maze::{GlobalState, MazeEdge, MazePos};
use crate::randomize::overworld::build::{BuildResult, SlotKind};
use crate::randomize::overworld::lock_keys::{ADVANCE_CPU, cell_key};
use crate::randomize::rom_data::{FS_ON_CLEAR, SND_LEVEL1UP, SOUND_QLEVEL1, STATUS_GLYPH_DIGIT0};
use crate::rom::Rom;

/// The pyramid: vanilla's 2-Pyramid map tile. It sits in page 1's completable
/// window (`$67`-`$69`), so the level-clear effect poofs it to an M/L panel
/// exactly as it does a numbered panel, and a marching Hammer Bro treats it as
/// an enterable node and hops over it (`Map_MarchValidateTravel`'s
/// `Tile_AttrTable` test).
pub(crate) const TILE_PYRAMID: u8 = 0x69;

/// Salt for [`roll_wands`]' substream: "WANDOM_1".
pub(crate) const WANDOM_SALT: u64 = 0x5741_4E44_4F4D_5F31;

/// Roll K: uniform over 1-7, capped at the airships the spine offers. With
/// `world_count < 7` the spine is shorter and fewer wands exist at all, which
/// is the same cap `maze_model` puts on a fixed K.
pub(crate) fn roll_wands<R: Rng>(spine_len: usize, rng: &mut R) -> u8 {
    let airships = spine_len.saturating_sub(1).min(usize::from(MAX_WANDS)) as u8;
    rng.random_range(1..=airships.max(1))
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
/// Shuffle-then-first is a uniform pick among a tier's qualifying levels that
/// usually stops after one or two solvability checks rather than running one
/// per level.
pub(crate) fn pick_pyramid<R: Rng>(state: &GlobalState, rng: &mut R) -> Option<MazePos> {
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
            if !reached.contains(&cell) {
                continue;
            }
            if state.spheres_with_blocked(&HashSet::from([cell])).solvable {
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

/// The on-clear routine with one client: the pyramid.
///
/// Entered from map operation 8's no-orb exit with the player's cell in
/// `T1`/`T2` (`$00`/`$01`) in [`cell_key`]'s encoding. Ends `JMP` back to the
/// effect's own `advance`, so a clear anywhere else costs four compares.
///
/// A second client is one more compare block before `done`: its branches are
/// relative and its exit is the shared `JMP`, so adding one moves nothing the
/// effect points at.
///
/// 41 reserved, 25 used.
fn on_clear_routine(pyramid: MazePos, k: u8) -> [u8; 25] {
    let [key0, key1] = cell_key(pyramid.0, pyramid.1);
    let [reveal_lo, reveal_hi] = WANDOM_REVEAL.to_le_bytes();
    let [snd_lo, snd_hi] = SOUND_QLEVEL1.to_le_bytes();
    let [adv_lo, adv_hi] = ADVANCE_CPU.to_le_bytes();
    #[rustfmt::skip]
    let code = [
        0xA5, 0x00,                              //  0: LDA <T1       ; key0 of the cell just cleared
        0xC9, key0,                              //  2: CMP #pyramid key0
        0xD0, 0x10,                              //  4: BNE done      (-> 22)
        0xA5, 0x01,                              //  6: LDA <T2       ; key1
        0xC9, key1,                              //  8: CMP #pyramid key1
        0xD0, 0x0A,                              // 10: BNE done      (-> 22)
        0xA9, STATUS_GLYPH_DIGIT0 + k,           // 12: LDA #digit K
        0x8D, reveal_lo, reveal_hi,              // 14: STA WANDOM_REVEAL   ; the readout draws this
        0xA9, SND_LEVEL1UP,                      // 17: LDA #SND_LEVEL1UP
        0x8D, snd_lo, snd_hi,                    // 19: STA Sound_QLevel1
        0x4C, adv_lo, adv_hi,                    // 22: JMP advance   <- done
    ];
    code
}

/// Write the on-clear routine. `install_on_clear` (in the lock stage, after the
/// effect itself is written) is what makes op 8 reach it.
pub(crate) fn apply(rom: &mut Rom, pyramid: MazePos, k: u8) {
    debug_assert!((1..=MAX_WANDS).contains(&k), "Wandom's K is 1-7");
    rom.write_range(FS_ON_CLEAR, &on_clear_routine(pyramid, k));
}

#[cfg(test)]
mod asm_checks {
    use super::*;
    use crate::randomize::overworld::lock_keys::ON_CLEAR_CPU;
    use crate::randomize::rom_data::{NMI_SAFE_MAX, asm};
    use mos6502::cpu::CPU;
    use mos6502::instruction::Ricoh2a03;
    use mos6502::memory::{Bus, Memory};

    #[test]
    fn on_clear_is_well_formed() {
        for k in 1..=MAX_WANDS {
            asm::check(&on_clear_routine((2, (4, 20)), k))
                .allocation(FS_ON_CLEAR)
                .origin(ON_CLEAR_CPU)
                .zero_page(NMI_SAFE_MAX, &[])
                .assert_ok();
        }
    }

    /// Run the routine on an emulated 2A03 with the player on `at`, and return
    /// what it left in the reveal byte and the sound queue.
    fn run(pyramid: MazePos, at: MazePos, k: u8) -> (u8, u8) {
        let code = on_clear_routine(pyramid, k);
        let mut mem = Memory::new();
        mem.set_bytes(ON_CLEAR_CPU, &code);
        let [k0, k1] = cell_key(at.0, at.1);
        mem.set_byte(0x0000, k0);
        mem.set_byte(0x0001, k1);
        let mut cpu = CPU::new(mem, Ricoh2a03);
        cpu.registers.program_counter = ON_CLEAR_CPU;
        for _ in 0..100 {
            if cpu.registers.program_counter == ADVANCE_CPU {
                return (cpu.memory.get_byte(WANDOM_REVEAL), cpu.memory.get_byte(SOUND_QLEVEL1));
            }
            cpu.single_step();
        }
        panic!("the on-clear routine never reached advance");
    }

    /// The pyramid's own cell reveals K; every other cell — same world, same
    /// row, same column, another world — falls straight through.
    #[test]
    fn only_the_pyramid_cell_reveals() {
        let pyramid = (2, (4, 20));
        for k in 1..=MAX_WANDS {
            assert_eq!(run(pyramid, pyramid, k), (STATUS_GLYPH_DIGIT0 + k, SND_LEVEL1UP));
        }
        for other in [(2, (4, 21)), (2, (5, 20)), (3, (4, 20)), (2, (4, 4)), (2, (6, 36))] {
            assert_eq!(run(pyramid, other, 5), (0, 0), "{other:?} revealed K");
        }
    }

    /// The routine's exit is the effect's `advance`, and that address has to
    /// still be `INC Map_Operation` — the one thing the prologue swap promises
    /// never to move.
    #[test]
    fn the_exit_is_advance() {
        let code = on_clear_routine((0, (0, 0)), 3);
        let [lo, hi] = ADVANCE_CPU.to_le_bytes();
        assert_eq!(&code[22..25], &[0x4C, lo, hi]);
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

    /// Read the pyramid's cell back out of the on-clear routine's two compares.
    fn decode_cell(rom: &Rom) -> MazePos {
        let key0 = rom.read_byte(FS_ON_CLEAR + 3);
        let key1 = rom.read_byte(FS_ON_CLEAR + 9);
        let world = usize::from(key0 & 0x0F);
        let row = usize::from(key0 >> 4) - 2;
        let col = usize::from(key1 >> 4) + 16 * usize::from(key1 & 0x0F);
        (world, (row, col))
    }

    /// **The whole feature on finished ROMs.** For each seed: op 8's no-orb
    /// exit goes to the on-clear routine; that routine is keyed on a cell that
    /// really carries the pyramid tile; the digit it reveals is the K the
    /// maze was built for; the readout is the hidden one; and the level is
    /// reachable and optional in the maze that shipped.
    #[test]
    fn wandom_hides_k_behind_a_real_optional_pyramid() {
        use crate::randomize::maze::LAST_MAZE;
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
            let cell = decode_cell(&rom);
            let (world, (row, col)) = cell;
            assert_eq!(
                rom.read_byte(map_tile_offset(world, row, col)),
                TILE_PYRAMID,
                "seed {seed}: the on-clear key names {cell:?}, which is not a pyramid"
            );
            let k = rom.read_byte(FS_ON_CLEAR + 13) - STATUS_GLYPH_DIGIT0;
            assert_eq!(k, state.wands_required, "seed {seed}: the reveal is not the gate's K");
            assert!((1..=MAX_WANDS).contains(&k));
            assert!(
                state.spheres().spheres.iter().any(|s| s.reached.contains(&cell)),
                "seed {seed}: the pyramid at {cell:?} cannot be reached"
            );
            assert!(
                state.spheres_with_blocked(&HashSet::from([cell])).solvable,
                "seed {seed}: the pyramid at {cell:?} is required"
            );
        }
    }

    /// A fixed-K maze seed leaves op 8's prologue exactly as it was.
    #[test]
    fn a_fixed_k_maze_is_not_hooked() {
        use crate::randomize::rom_data::FS_FORTRESS_FX;
        let Some(mut rom) = vanilla() else {
            eprintln!("SKIP: requires the ROM");
            return;
        };
        let options = crate::Options { maze_wands: 3, ..wandom_options() };
        crate::pipeline::randomize(&mut rom, 1, &options);
        let [lo, hi] = ADVANCE_CPU.to_le_bytes();
        assert_eq!(rom.read_range(FS_FORTRESS_FX + 12, 3), &[0x4C, lo, hi]);
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
            let cell = pick_pyramid(&state, &mut rng);
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

    #[test]
    fn rolls_stay_in_range_and_cover_it() {
        use rand::SeedableRng;
        let mut rng = rand_chacha::ChaCha8Rng::seed_from_u64(1);
        let mut seen = [false; 8];
        for _ in 0..500 {
            let k = roll_wands(8, &mut rng);
            assert!((1..=7).contains(&k));
            seen[usize::from(k)] = true;
        }
        assert!(seen[1..=7].iter().all(|&s| s), "every K from 1 to 7 should come up");
        // A two-world spine has one airship, so one wand at most.
        assert_eq!(roll_wands(2, &mut rng), 1);
    }
}
