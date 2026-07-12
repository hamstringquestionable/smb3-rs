//! Top-level randomization orchestration: parse options, run the pipeline, and
//! stamp the result. The `Options` config and flag-key codec live in submodules.

use rand::SeedableRng;
use rand::seq::IndexedRandom;
use rand_chacha::ChaCha8Rng;

use crate::randomize;
use crate::rom::Rom;

mod flag_key;
mod options;

use flag_key::*;
use options::*;

// Public API re-exported by the crate root (see lib.rs).
pub use options::{
    EnemyMode, FireFlowerMode, Options, PiranhaMode, Tri, ITEM_RANDOM,
    ITEM_RANDOM_NO_WHISTLE, ITEM_RANDOM_SUIT_ONLY, STARTING_LIVES_VALUES,
};

#[cfg(test)]
mod tests;

/// Free space in PRG012 after the Big ? Block trampoline (0x19DD0 region).
/// The trampoline uses 0x19DD0–0x19DE1; we place the 16-byte stamp at 0x19DF0.
const STAMP_OFFSET: usize = 0x19DF0;

/// Resolve a starting item value: sentinels (14/15/16) become random concrete
/// items; concrete values (0–13) pass through unchanged.
pub fn resolve_starting_item(item: u8, rng: &mut ChaCha8Rng) -> u8 {
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
        _ => item,
    }
}

/// Apply all enabled randomizations to a ROM using the given seed.
pub fn randomize(rom: &mut Rom, seed: u64, options: &Options) {
    randomize_inner(rom, seed, options, None);
}

/// Same as [`randomize`] but additionally captures a snapshot of the overworld
/// `BuildResult` right before the writer stamps it onto the ROM. Used by
/// internal analyzer tests (and the future WASM single-seed dump endpoint) to
/// inspect the exact topology the player will see, while still consuming RNG
/// in the same order as a real playthrough.
#[allow(dead_code)] // consumed by overworld_build::tests::test_dump_required_progression.
pub(crate) fn randomize_with_overworld_capture(
    rom: &mut Rom,
    seed: u64,
    options: &Options,
    capture: &mut Option<randomize::overworld_build::BuildResult>,
) {
    randomize_inner(rom, seed, options, Some(capture));
}

fn randomize_inner(
    rom: &mut Rom,
    seed: u64,
    options: &Options,
    overworld_capture: Option<&mut Option<randomize::overworld_build::BuildResult>>,
) {
    let mut rng = ChaCha8Rng::seed_from_u64(seed);

    // Resolve random starting items up front (deterministic from seed)
    let resolved_items: Vec<u8> = options.starting_items.iter()
        .map(|&item| resolve_starting_item(item, &mut rng))
        .collect();

    // Resolve the player-hidden tri-state flags up front. These draw from a
    // dedicated substream (MAYBE_SALT) so flipping a flag to `Maybe` never
    // perturbs the main `rng` sequence — a seed with no `Maybe` flags is
    // byte-identical to before this feature. The order here is part of the
    // determinism contract: do not reorder, and append any future tri flags
    // at the end.
    let mut maybe_rng = ChaCha8Rng::seed_from_u64(seed ^ MAYBE_SALT);
    let hammer_breaks_locks = options.hammer_breaks_locks.resolve(&mut maybe_rng);
    let hammer_breaks_bridges = options.hammer_breaks_bridges.resolve(&mut maybe_rng);
    let troll_pipes = options.troll_pipes.resolve(&mut maybe_rng);
    let more_hammer_rocks = options.more_hammer_rocks.resolve(&mut maybe_rng);
    let eights_are_wild = options.eights_are_wild.resolve(&mut maybe_rng);
    let antechamber_shuffle = options.antechamber_shuffle.resolve(&mut maybe_rng);

    // QoL map patches run first so all subsequent overworld operations
    // (fortress redistribution, pipe shuffle, lock shuffle) see the final
    // map connectivity and store correct replacement tiles.
    rom.set_tag("qol/drawbridges");
    randomize::qol::fix_w3_drawbridges(rom);
    // Path-blocking rocks (W2 secret path, W3 boat dock, W4 pipe shortcut) are
    // always removed: the overworld builder relies on those tiles being open
    // for connectivity, so this is no longer player-gated.
    rom.set_tag("qol/rocks");
    randomize::qol::remove_rocks(rom);
    if more_hammer_rocks {
        rom.set_tag("qol/more_hammer_rocks");
        randomize::qol::make_hammer_rocks(rom);
    }

    // W8 Dark World map edits. The screen-3 water/bridge final page is always
    // applied; the screen-0 canoe + screen-2 extra paths are gated behind
    // `8s are Wild`. Both must run before the overworld builder so it sees the
    // new connectivity.
    rom.set_tag("qol/w8_bridges");
    randomize::qol::apply_w8_bridges(rom);
    if eights_are_wild {
        rom.set_tag("qol/w8_canoe_and_paths");
        randomize::qol::apply_w8_canoe_and_paths(rom);
    }

    // Fix Big ? Block bonus rooms so they follow the level, not the world slot.
    // Always applied — needed whenever world_order or cross-world shuffle is active,
    // and harmless (identity mapping) when worlds aren't shuffled.
    rom.set_tag("qol/big_q_blocks");
    randomize::qol::fix_big_q_block_rooms(rom);

    // Autoscroll must run BEFORE powerups and the overworld builder:
    // it writes pre-baked replacement level data for airship levels, and
    // powerups/enemies need to randomize on top of that patched data.
    // It also writes airship pointer table redirects to hardcoded vanilla
    // offsets — the overworld builder's resort_pointer_table() rearranges
    // entries later, so autoscroll must go first.
    if options.disable_autoscroll {
        rom.set_tag("autoscroll");
        randomize::autoscroll::disable_autoscroll(rom);
    }
    // Beta stage layout fixes must run before powerups/enemies so the
    // randomization passes see the patched bytes (some patches reshape
    // commands or convert hidden powerblocks into randomizable shapes).
    if options.include_beta_stages {
        rom.set_tag("qol/beta_stages");
        randomize::qol::fix_beta_stages(rom);
    }
    if options.powerups {
        rom.set_tag("powerups");
        randomize::powerups::randomize(rom, &mut rng, options.hammer_vulnerable_koopalings);
    }
    // Player colors and world colors are independent cosmetic layers:
    // `palettes` drives the character wardrobe (random or player-picked),
    // `palette_themed` drives level/enemy/map palettes.
    if options.palettes || options.palette_themed {
        rom.set_tag("palettes");
        let mut palette_rng = ChaCha8Rng::from_os_rng();
        if options.palettes {
            randomize::palettes::randomize(rom, &mut palette_rng, options.player_color);
        }
        if options.palette_themed {
            randomize::palettes::randomize_themed(rom, &mut palette_rng);
        }
    }
    if options.any_enemies_active() {
        rom.set_tag("enemies");
        randomize::enemies::randomize(rom, &mut rng, options);
    }
    // Force one of β9's Fire Chomps into a Tornado. Runs after the enemy pass
    // (which may randomize β9's Fire Chomps when it's placed) so the Tornado is
    // final, and only when beta stages are included so we don't touch β9's data
    // for nothing.
    if options.include_beta_stages {
        rom.set_tag("beta_tornado");
        randomize::beta_tornado::randomize_beta9_tornado(rom, &mut rng);
    }
    randomize::bowser_castle::randomize(rom, &mut rng);
    randomize::podoboo_gauntlet::randomize(rom, &mut rng);
    if options.world_order {
        rom.set_tag("world_order");
        randomize::world_order::randomize(rom, &mut rng, options.world_count);
    }
    if options.big_q_blocks {
        rom.set_tag("enemies/big_q_blocks");
        randomize::enemies::randomize_big_q_blocks(rom, &mut rng);
    }
    // Airship shuffle runs after autoscroll (which patches airship pointer
    // entries at vanilla indices) and before the overworld builder (whose
    // resort_pointer_table re-sorts everything). shuffle_entries only moves
    // tileset + ObjSets + LevelLayouts, preserving row/col position, so
    // patched data travels correctly to its new world.
    if options.shuffle_airships {
        rom.set_tag("levels/airships");
        randomize::levels::randomize_airships(rom, &mut rng);
    }

    // Antechamber shuffle touches only level data (entry headers + junction
    // commands), never pointer tables or enemy streams, so it's independent
    // of the overworld builder and the enemy/powerup passes.
    if antechamber_shuffle {
        rom.set_tag("levels/antechambers");
        randomize::antechambers::shuffle(rom, &mut rng);
    }

    // Koopaling stability patches — needed whenever Koopalings may load in a
    // non-native world (airship shuffle, identity remap) or when the hammer
    // vulnerability patch is applied. Covers the softlock fix plus Fred's
    // three guards (phantom double-stomps, stale VRAM writes, Y wraparound).
    let koopalings_may_travel = options.shuffle_airships
        || options.hammer_vulnerable_koopalings
        || options.random_koopalings;
    if koopalings_may_travel {
        rom.set_tag("koopalings/fix_softlock");
        randomize::koopalings::fix_koopaling_softlock(rom);
        rom.set_tag("koopalings/collision_guard");
        randomize::koopalings::koopaling_collision_guard(rom);
        rom.set_tag("koopalings/vram_clear");
        randomize::koopalings::koopaling_vram_clear(rom);
        rom.set_tag("koopalings/y_clamp");
        randomize::koopalings::koopaling_y_clamp(rom);
    }

    // Make Koopalings vulnerable to thrown hammers (PRG000 $8302).
    if options.hammer_vulnerable_koopalings {
        rom.set_tag("koopalings/hammer_vulnerable");
        randomize::koopalings::hammer_vulnerable_koopalings(rom);
    }

    // Random Koopaling identity remap (Fred's Map_Unused7EEA hijack).
    if options.random_koopalings {
        rom.set_tag("koopalings/random_identity");
        randomize::koopalings::random_koopalings(rom, &mut rng);
    }

    rom.set_tag("overworld/builder");
    let mut catalog = randomize::node_catalog::NodeCatalog::build(rom, options.include_beta_stages);
    // Piranha shuffle: free the two W7 plant levels into the pool. The sprite
    // clear must precede the builder — capacity/eligibility reads sprite
    // state straight from the ROM.
    let piranha_active = options.piranha_shuffle != PiranhaMode::Off;
    if piranha_active {
        rom.set_tag("piranha_shuffle");
        randomize::piranha_rooms::clear_vanilla_plants(rom);
        catalog.release_map_objects();
    }
    if options.swap_start_airship {
        randomize::start_airship_swap::pick_swaps(&mut catalog, &mut rng);
    }
    let pickup = randomize::overworld_pickup::pick_up(
        rom,
        &catalog,
        randomize::overworld_pickup::PickupFlags {
            shuffle_spade_games: options.shuffle_spade_games,
            shuffle_toad_houses: options.shuffle_toad_houses,
            shuffle_hammer_bros: options.shuffle_hammer_bros,
        },
    );
    let data = randomize::overworld_build::OverworldData {
        pickup: &pickup,
        catalog: &catalog,
    };
    let mut build = randomize::overworld_build::build(
        rom,
        &data,
        &mut rng,
        randomize::overworld_build::BuildFlags {
            shuffle_toad_houses: options.shuffle_toad_houses,
            eights_are_wild,
            shuffle_hammer_bros: options.shuffle_hammer_bros,
        },
    );
    if options.hands_levels {
        rom.set_tag("hands_levels");
        randomize::hands_levels::mark_hand_traps(&mut build, &mut rng);
        randomize::hands_levels::install_full_grab(rom);
    }
    if troll_pipes {
        rom.set_tag("troll_pipes");
        randomize::troll_pipes::mark_troll_pipes(&mut build, &mut rng);
    }
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
    randomize::overworld_writer::write_overworld(
        rom,
        &build,
        &data,
        &mut rng,
        randomize::overworld_writer::WriteFlags {
            cross_world: true,
            shuffle_hammer_bros: options.shuffle_hammer_bros,
            piranha: options.piranha_shuffle,
        },
    );
    // Give each W8 Hand its own treasure-room enemy stream so the chest
    // randomizer can roll a unique item per Hand. Runs before items::randomize
    // so the cloned Y-bytes are in place when chests roll.
    rom.set_tag("hand_rooms");
    randomize::hand_rooms::patch_clone_hand_rooms(rom);

    // Piranha shuffle: once 7-P1/7-P2 leave their vanilla map-object spots
    // they can be entered like any level tile, so their chests must carry
    // their own OBJ_TREASURESET. Runs before items::randomize so the cloned
    // item bytes are in place when chests roll.
    if piranha_active {
        rom.set_tag("piranha_rooms");
        randomize::piranha_rooms::install_treasure_sets(rom);
    }

    if options.chest_items {
        rom.set_tag("items");
        randomize::items::randomize(rom, &mut rng, options.remove_whistles, piranha_active);
    } else if options.remove_whistles {
        rom.set_tag("items/whistles");
        randomize::items::remove_whistles_only(rom, &mut rng);
    }

    // Set starting lives (patched later by starting_items trampoline if items present)
    rom.set_tag("qol/starting_lives");
    randomize::qol::set_starting_lives(rom, options.starting_lives);

    // Anchors stay in inventory as mystery items — patch the item-use
    // dispatch so using an anchor triggers a random powerup effect.
    rom.set_tag("items/mystery_anchor");
    randomize::items::write_mystery_anchor(rom, &mut rng);

    // Patch double-digit level tiles (11–19) to show a "1" tens digit
    rom.set_tag("metatile/double_digit");
    randomize::overworld_writer::patch_double_digit_metatiles(rom);

    // Freeze metatile 0x6A's CHR animation so it can serve as a static fortress tile.
    rom.set_tag("metatile/6a_freeze");
    randomize::overworld_writer::patch_metatile_6a_freeze(rom);

    // Randomize king quotes (always on — cosmetic flavor text)
    rom.set_tag("king_quotes");
    randomize::king_quotes::randomize(rom, &mut rng);

    // Cosmetic: render every item visual (reserve grid, Toad House chests,
    // in-level treasure boxes) as the Anchor sprite.
    if options.anchor_visuals {
        rom.set_tag("anchor_visuals");
        randomize::anchor_visuals::apply(rom);
    }

    // Skip the wand falling cutscene after defeating a Koopaling.
    if options.skip_wand_cutscene {
        rom.set_tag("koopalings/skip_wand_cutscene");
        randomize::koopalings::skip_wand_cutscene(rom);
    }

    // Remove N-card (N-Spade) panels from the overworld map.
    if options.remove_n_cards {
        rom.set_tag("qol/remove_n_cards");
        randomize::qol::remove_n_cards(rom);
    }

    // Fix canoe softlocks. Always applied: the vanilla W3 canoe is always
    // present (and the W8 canoe is present when `8s are Wild` is on), and canoes
    // are also reachable via spade and toad-house shuffle. The fix is
    // world-agnostic (keys on the dock tile 0x4B and canoe object 0x10), so
    // running it unconditionally is correct and safe.
    rom.set_tag("qol/fix_canoe_softlock");
    randomize::qol::fix_canoe_softlock(rom);

    // Adjust Bowser and Koopaling hitboxes.
    if options.adjust_boss_hitboxes {
        rom.set_tag("koopalings/adjust_boss_hitboxes");
        randomize::koopalings::adjust_boss_hitboxes(rom);
    }

    // Per-Koopaling random stomp counts (1–5 hits each).
    if options.koopaling_hits {
        rom.set_tag("koopalings/random_hits");
        randomize::koopalings::randomize_koopaling_hits(rom, &mut rng);
    }

    // Per-fortress Boom-Boom random stomp counts (1–5 hits each).
    if options.boomboom_hits {
        rom.set_tag("boomboom/random_hits");
        randomize::koopalings::randomize_boomboom_hits(rom, &mut rng);
    }

    // Hammer breaks tiles on the overworld map (locks, bridges, or both).
    if hammer_breaks_locks || hammer_breaks_bridges {
        rom.set_tag("qol/hammer_breaks_tiles");
        randomize::qol::hammer_breaks_tiles(rom, hammer_breaks_locks, hammer_breaks_bridges);
    }

    // MaCobra52's "Early Sun" — Angry Sun begins attacking immediately.
    if options.early_sun {
        rom.set_tag("qol/early_sun");
        randomize::qol::apply_early_sun(rom);
    }

    // "Limit Bro Movement" — gate the wandering Hammer Bros' overworld roaming.
    if options.limit_bro_movement {
        rom.set_tag("qol/limit_bro_movement");
        randomize::qol::apply_limit_bro_movement(rom);
    }

    // MaCobra52's "Japanese damage system" — damage drops straight to Small
    // Mario (or kills from a suit) instead of tier-by-tier demotion.
    if options.japanese_damage {
        rom.set_tag("qol/japanese_damage");
        randomize::qol::apply_japanese_damage(rom);
    }

    // MaCobra52's "Infinite use Mushroom Houses" — toad houses don't get
    // removed from the map after entering, so they're reusable.
    if options.infinite_mushroom_houses {
        rom.set_tag("qol/infinite_mushroom_houses");
        randomize::qol::apply_infinite_mushroom_houses(rom);
    }

    // MaCobra52's "Fast Mushroom House" — skip entry input-lock + faster exit.
    if options.fast_mushroom_house {
        rom.set_tag("qol/fast_mushroom_house");
        randomize::qol::apply_fast_mushroom_house(rom);
    }

    // MaCobra52's "Faster Tail Speed" — reduced tail slowdown + balancing
    // flight-time cut and 7-6 wall adjustment.
    if options.faster_tail_speed {
        rom.set_tag("qol/faster_tail_speed");
        randomize::qol::apply_faster_tail_speed(rom);
    }

    // MaCobra52's "No Game Over Penalty" — keep reserve inventory and
    // map progress after a Game Over.
    if options.no_game_over_penalty {
        rom.set_tag("qol/no_game_over_penalty");
        randomize::qol::apply_no_game_over_penalty(rom);
    }

    // Card speed clear: one-of-each clears cards with +1 life but no cutscene.
    if options.card_speed_clear {
        rom.set_tag("qol/card_speed_clear");
        randomize::qol::card_speed_clear(rom);
    }

    // Title screen seed hash icons (cosmetic verification).
    // This hooks STA $0736 at 0x308E2 for intro skip.
    // Skipped when the user opted out of ROM validation, since the hooks
    // assume vanilla offsets in PRG031 that may have been changed by a mod.
    if !options.skip_rom_validation {
        rom.set_tag("title_screen");
        randomize::title_screen::write_seed_hash(rom, seed, options);
    }

    // Starting items trampoline — must run AFTER title_screen because both
    // write to the lives init region at 0x308E0: this one wins, overwriting
    // title_screen's intro-skip hook at 0x308E2. The trampoline replays the
    // identical intro-skip + menu-music bytes (shared
    // `title_screen::intro_skip_music_bytes`), so behavior is unchanged;
    // title_screen's FS_INTRO_SKIP routine is left in ROM unreferenced.
    if !options.starting_items.is_empty() {
        rom.set_tag("qol/starting_items");
        randomize::qol::write_starting_items(rom, seed, options.starting_lives, &resolved_items);
    }

    // MaCobra patches — always-on bugfixes and fairness tweaks.
    rom.set_tag("qol/macobra");
    randomize::qol::apply_macobra_patches(rom);

    // Faster Frog — speeds up Frog-Suit swimming. MUST run after
    // apply_macobra_patches: two of its writes patch inside the tail-swim
    // routine that macobra writes unconditionally, so it has to layer on top.
    if options.faster_frog {
        rom.set_tag("qol/faster_frog");
        randomize::qol::apply_faster_frog(rom);
    }

    // Random Fire Flower — in-level Fire Flower grants a position-derived suit
    // instead of always Fire. Pure static patch (no RNG): the substitution is a
    // deterministic function of World_Num + the flower's level position.
    if options.fire_flower != FireFlowerMode::Off {
        rom.set_tag("fire_flower");
        randomize::fire_flower::apply(rom, options.fire_flower);
    }

    // Stamp flag key + seed into free space at STAMP_OFFSET (PRG012):
    //   "S3R" magic + version byte, the flag key bytes (13 in v23), then the
    //   seed (little-endian u64). Sizes derive from to_flag_bytes() so the
    //   stamp grows with the flag key.
    rom.set_tag("stamp");
    let flag_bytes = options.to_flag_bytes();
    let mut stamp = Vec::with_capacity(4 + flag_bytes.len() + 8);
    stamp.extend_from_slice(b"S3R");
    stamp.push(FLAG_KEY_VERSION);
    stamp.extend_from_slice(&flag_bytes);
    stamp.extend_from_slice(&seed.to_le_bytes());
    rom.write_range(STAMP_OFFSET, &stamp);
}
