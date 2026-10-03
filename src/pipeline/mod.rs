//! The pipeline: vanilla ROM in, randomized ROM out. `randomize_inner` is its
//! table of contents and `stages` holds each step. The `Options` config and
//! flag-key codec live in submodules.

use rand::SeedableRng;
use rand::seq::IndexedRandom;
use rand_chacha::ChaCha8Rng;

use crate::randomize;
use crate::rom::Rom;

mod flag_key;
mod options;
mod stages;

use flag_key::*;
use options::*;

// Public API re-exported by the crate root (see lib.rs).
pub use flag_key::{current_flag_key_version, flag_key_fields, flag_key_version_of};
pub use options::{
    DejaVuMode, EnemyMode, FireFlowerMode, HazardLimit, HintMode, ITEM_RANDOM,
    ITEM_RANDOM_NO_SUITS, ITEM_RANDOM_NO_WHISTLE, ITEM_RANDOM_SUIT_ONLY, ITEMS, Options,
    PiranhaMode, STARTING_LIVES_VALUES, Tri, WildChaser, item_display_name, item_id,
};

#[cfg(test)]
mod tests;

/// Free space in PRG012 after the Big ? Block trampoline (0x19DD0 region).
/// The trampoline uses 0x19DD0–0x19DE1; we place the 16-byte stamp at 0x19DF0.
use crate::randomize::rom_data::FS_SEED_STAMP as STAMP_OFFSET;

/// Resolve a starting item value: sentinels (14–17) become random concrete
/// items; concrete values (0–13) pass through unchanged.
///
/// `whistles_removed` is the effective value, [`stages::Resolved::whistles_removed`]. Only
/// [`ITEM_RANDOM_NO_SUITS`] reads it — the other pools predate the flag and
/// keep their historical contents, so passing it does not move any seed that
/// does not use the new sentinel.
pub fn resolve_starting_item(item: u8, whistles_removed: bool, rng: &mut ChaCha8Rng) -> u8 {
    match item {
        ITEM_RANDOM => {
            // Any item 1–13
            let pool: Vec<u8> = (1..=13).collect();
            *pool.choose(rng).unwrap()
        }
        ITEM_RANDOM_NO_WHISTLE => {
            // Any item 1–13 except whistle (0x0C)
            let pool: Vec<u8> = (1..=13).filter(|&v| v != 0x0C).collect();
            *pool.choose(rng).unwrap()
        }
        ITEM_RANDOM_SUIT_ONLY => {
            // Suits only: mushroom(1) through hammer suit(6)
            let pool: Vec<u8> = (1..=6).collect();
            *pool.choose(rng).unwrap()
        }
        ITEM_RANDOM_NO_SUITS => {
            // Utility items only. A curated list, not the complement of the
            // suit pool: P-Wing (0x08) and Anchor (0x0A) are not suits and are
            // still out. Whistle joins only when the seed hands whistles out at
            // all — otherwise this pool would be the one place a mode that
            // removes whistles still starts you with one.
            let mut pool: Vec<u8> = vec![0x07, 0x09, 0x0B, 0x0D];
            if !whistles_removed {
                pool.push(0x0C);
            }
            *pool.choose(rng).unwrap()
        }
        _ => item,
    }
}

/// Apply all enabled randomizations to a ROM using the given seed.
pub fn randomize(rom: &mut Rom, seed: u64, options: &Options) {
    randomize_inner(rom, seed, options, None);
}

/// Same as [`randomize()`] but additionally captures a snapshot of the overworld
/// `BuildResult` right before the writer stamps it onto the ROM. Used by
/// internal analyzer tests (and the future WASM single-seed dump endpoint) to
/// inspect the exact topology the player will see, while still consuming RNG
/// in the same order as a real playthrough.
#[allow(dead_code)] // consumed by crate::randomize::overworld::build::tests::test_dump_required_progression.
pub(crate) fn randomize_with_overworld_capture(
    rom: &mut Rom,
    seed: u64,
    options: &Options,
    capture: &mut Option<randomize::overworld::build::BuildResult>,
) {
    randomize_inner(rom, seed, options, Some(capture));
}

/// The whole pipeline, as a table of contents: vanilla ROM in, randomized ROM
/// out. Each step is one function in [`stages`], run in this order — read down
/// the list to find where something happens, then follow the call.
///
/// **The order is part of the seed.** Every stage handed `rng` draws from one
/// stream, so moving a call changes every seed downstream of it. Two side
/// streams keep subsystems from perturbing it: `item_rng` (see `ITEM_SALT`) for
/// the item tables, and the `MAYBE_SALT` stream that [`stages::resolve`]
/// consumes for the tri-state flags. Palettes draw from OS entropy instead, so
/// they are deliberately *not* reproducible from the seed.
///
/// **Some options override others**, and each override sits in a different
/// stage, so here they are in one place:
/// - `world_maze` forces `world_order` on and pins `world_count` to 7
///   ([`stages::world_order_and_shuffles`]), forces whistles out of the item
///   pools ([`stages::resolve`]), and forces `no_game_over_penalty` on
///   ([`stages::engine_patches`]).
/// - Without `world_maze`, `hints` and `item_gates` are treated as off
///   ([`stages::resolve`]).
/// - `king_quotes` gates only its ROM writes; the quotes are always drawn, so
///   toggling it moves no seed ([`stages::bosses_and_quotes`]).
fn randomize_inner(
    rom: &mut Rom,
    seed: u64,
    options: &Options,
    overworld_capture: Option<&mut Option<randomize::overworld::build::BuildResult>>,
) {
    let mut rng = ChaCha8Rng::seed_from_u64(seed);
    // Chest/Toad House/Hammer Bro/letter items draw from their own substream
    // for the same reason (see `ITEM_SALT`): they are rolled before the
    // overworld pickup now, and a substream keeps that out of the main
    // sequence so no map moves.
    let mut item_rng = ChaCha8Rng::seed_from_u64(seed ^ ITEM_SALT);

    // 0. Starting items and tri-state flags, settled before any ROM write.
    let run = stages::resolve(seed, options, &mut rng);

    // 1-4. Level and map data: fixes the later passes build on, then the
    //      per-level randomizers, world order, and the Koopalings.
    stages::map_fixes(rom, &run);
    stages::level_data(rom, options, &mut rng);
    let progression = stages::world_order_and_shuffles(rom, options, &run, &mut rng);
    stages::koopalings(rom, options, &mut rng);

    // 5-8. The overworld and maze *model*: nothing here writes a map grid.
    let catalog = stages::overworld_catalog(rom, options, &run, &mut rng);
    stages::item_tables(rom, options, &run, &mut item_rng);
    let (pickup, mut build) = stages::overworld_build(rom, options, &run, &catalog, &mut rng);
    let maze = stages::maze_model(rom, options, &run, &mut build, progression.as_deref(), &mut rng);

    // --- OVERWORLD CAPTURE POINT ---
    // Hand a clone of the finalized BuildResult (post hands/troll mutations,
    // pre-writer) to any caller that asked for it. Used by the progression
    // analyzer to inspect the topology the player will actually see, with
    // RNG consumed exactly as in a real playthrough. Keep this immediately
    // before `write_overworld` so future randomization steps inserted after
    // the writer don't pollute the snapshot.
    if let Some(slot) = overworld_capture {
        *slot = Some(build.clone());
    }

    // 9-12. Write the map, then everything that reads the finished map.
    let written = stages::write_overworld(rom, options, &run, &build, &pickup, &catalog, &mut rng);
    let canoe_gated = stages::maze_rom(rom, options, maze, &written, progression.as_deref());
    stages::locks(rom, &build, &written);
    stages::after_the_map(rom, options, &written, progression.as_deref(), &mut rng);

    // 13-17. Bosses and quotes, engine patches, title screen, stamp.
    stages::bosses_and_quotes(rom, options, &written, progression.as_deref(), &mut rng);
    stages::engine_patches(rom, options, &run, &written, canoe_gated, &mut rng);
    stages::title_and_starting_items(rom, seed, options, &run);
    stages::macobra_layer(rom, options);
    stages::stamp(rom, seed, options);
}
