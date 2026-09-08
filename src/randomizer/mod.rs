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
pub use flag_key::{current_flag_key_version, flag_key_fields, flag_key_version_of};
pub use options::{
    DejaVuMode, EnemyMode, FireFlowerMode, HazardLimit, HintMode, ITEM_RANDOM,
    ITEM_RANDOM_NO_WHISTLE, ITEM_RANDOM_SUIT_ONLY, ITEMS, Options, PiranhaMode,
    STARTING_LIVES_VALUES, Tri, WildChaser, item_display_name, item_id,
};

#[cfg(test)]
mod tests;

/// Free space in PRG012 after the Big ? Block trampoline (0x19DD0 region).
/// The trampoline uses 0x19DD0–0x19DE1; we place the 16-byte stamp at 0x19DF0.
use crate::randomize::rom_data::FS_SEED_STAMP as STAMP_OFFSET;

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
    let resolved_items: Vec<u8> =
        options.starting_items.iter().map(|&item| resolve_starting_item(item, &mut rng)).collect();
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

    // W1 shortcut rock. The tiles land either way — only the rock's
    // breakability follows `more_hammer_rocks`, so the map never leaks how a
    // `Maybe` roll came out. Before the builder, like every map edit.
    rom.set_tag("qol/w1_shortcut");
    randomize::qol::apply_w1_shortcut(rom, more_hammer_rocks);

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
    // World order shuffle. The credits reorder that aligns the ending montage
    // with the progression runs later (after the mini-maps are regenerated and
    // repacked, since it permutes the picture pointers those steps rewrite).
    // The world maze reads `world_order`'s table as its airship spine and
    // chains the wand counter through the routine it installs, so it cannot run
    // without it. Forced here rather than only in the CLI, because a flag key
    // can name `world_maze` with `world_order` off.
    let credits_progression = if options.world_order || options.world_maze {
        rom.set_tag("world_order");
        Some(randomize::world_order::randomize(rom, &mut rng, options.world_count))
    } else {
        None
    };
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
        randomize::antechambers::shuffle(
            rom,
            &mut rng,
            options.include_beta_stages,
            options.friendlier_levels,
        );
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
    let data = randomize::overworld_build::OverworldData { pickup: &pickup, catalog: &catalog };
    let mut build = randomize::overworld_build::build(
        rom,
        &data,
        &mut rng,
        randomize::overworld_build::BuildFlags {
            shuffle_toad_houses: options.shuffle_toad_houses,
            eights_are_wild,
            shuffle_hammer_bros: options.shuffle_hammer_bros,
            world_maze: options.world_maze,
        },
    );
    if options.hands_levels {
        rom.set_tag("hands_levels");
        randomize::hands_levels::mark_hand_traps(&mut build, &mut rng);
        randomize::hands_levels::install_full_grab(rom);
    }
    if troll_pipes {
        // No `set_tag` here: this only mutates `build`, and the ROM writes it
        // leads to happen later in the writer. Tagging it would label none of
        // its own bytes and leak the name onto everything the writer emits.
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
    rom.set_tag("overworld_writer");
    let lock_pairing = randomize::overworld_writer::write_overworld(
        rom,
        &build,
        &data,
        &mut rng,
        randomize::overworld_writer::WriteFlags {
            shuffle_hammer_bros: options.shuffle_hammer_bros,
            piranha: options.piranha_shuffle,
            friendlier_levels: options.friendlier_levels,
            deja_vu: options.deja_vu,
            deja_vu_forts: options.deja_vu_forts,
        },
    );

    // Set by the world maze, read by `lock_keys` below. When the maze runs it
    // owns the WHOLE lock/fortress assignment, not just the cross-world half —
    // its fill starts from the builder's pairing and swaps from there, so the
    // two cannot be mixed. `None` means no maze ran and the builder's pairing
    // stands.
    let mut maze_lock_keys: Option<Vec<randomize::lock_keys::LockEntry>> = None;

    // World maze: the eight world maps stop being a sequence and become the
    // rooms of one Metroidvania — telepads between them, a fortress that can
    // bust a lock in another world, and map progress that survives leaving.
    //
    // **The order inside this block is the whole of its correctness.** The
    // packed completion store derives its stencil from the map grids as they
    // finally stand, so every write that changes a grid runs first — the pad
    // tiles and the wand gate — and `lock_keys` runs last (just past the end of
    // this block), because it asks the packer where a given cell's bit lives
    // rather than re-deriving that arithmetic.
    //
    // Today the two grid writers are in fact bit-neutral, so only the tail of
    // that order is load-bearing. A cell claims a completion bit by being in
    // `Map_Removable_Tiles` or `Map_Completable_Tiles`, and `TILE_TELEPAD` and
    // `WAND_GATE_TILE` are in neither — `the_pad_tile_is_in_no_registry` and
    // `the_gate_tile_is_in_no_registry` pin that — while the cells they
    // overwrite are hammer-bro slots, blanks and the W8 bridge, which are in
    // neither either. The head of the order is kept anyway: it costs nothing,
    // and the day someone picks a tile that does claim a bit, the alternative
    // is a stencil that disagrees with the map by one bit somewhere past the
    // world it happened in.
    if options.world_maze {
        rom.set_tag("world_maze");
        // The spine IS `world_order`'s table, which is why the mode forces it
        // on. With `world_count < 7` it is shorter than eight, and the worlds
        // it leaves out are reachable only by telepad — see the design doc.
        let spine: Vec<usize> = credits_progression
            .as_ref()
            .expect("world_maze forces world_order on")
            .iter()
            .map(|&w| w as usize)
            .collect();
        // K cannot exceed the airships the spine offers: a shorter spine means
        // fewer than seven wands exist in the game at all.
        let wands = options.maze_wands.min(spine.len().saturating_sub(1) as u8);
        // Which fortress slot `assign_pool` gave 1-F, so the fill leaves that
        // one pairing alone. It picks uniformly among the slots the builder
        // marked `secret_exit_safe`, with its own RNG, so this has to be read
        // back rather than re-derived — and without it the fill re-pairs the
        // fortress with a lock nobody ever vetted (measured: the lock moved in
        // 55 of 60 seeds, and in 7 of 60 the secret exit ended the run).
        let one_f = lock_pairing
            .one_f_slot(&data)
            .map(|(world, section)| randomize::maze::FortRef { world, section });
        let (state, _report) = randomize::maze::generate(
            &build,
            &spine,
            wands,
            &randomize::maze::graph::Knobs::default(),
            one_f,
            &mut rng,
        );
        // **The maze owns the whole lock/fortress assignment, not half of it.**
        //
        // `fill` starts from the overworld builder's pairing — every lock opened
        // by a fortress in its own world — and moves by *swapping* the forts of
        // two locks, keeping a swap only while the maze stays solvable. So the
        // result is a permutation of the builder's, and it is a bijection at
        // every step: one lock per fortress, the charter's map-legibility rule.
        //
        // Taking only the cross-world half of that and leaving the rest to the
        // builder's original pairing loses every swap that happened to leave
        // both locks in their own worlds — 33.1% of same-world locks over 60
        // seeds — and lets a fortress that kept a stale local lock while gaining
        // a foreign one open two. Both halves travel together or neither does.
        //
        // **`maze::generate` does not need to be here.** It takes a
        // `BuildResult` and no `&Rom` — it is a pure function of the builder's
        // model — so the decision could be made before `write_overworld` runs.
        // What forces this block to sit *after* the writer is the writing:
        // `stamp_pad_tiles` and `wand_gate::apply` lay tiles over the grids the
        // writer just committed, and `world_persist` derives the packed store's
        // stencil from the result. The decision is only here because it is next
        // to its own writes.
        //
        // That is worth revisiting, because it is what makes the builder's
        // placement guarantees un-repairable: the map is already on the
        // cartridge by the time the fill permutes the array those guarantees
        // live in (see the 1-F secret-exit case). It consumes no RNG, so
        // nothing downstream shifts either way.
        maze_lock_keys = Some(randomize::maze::writer::lock_keys(&state));

        // Before `world_persist` for the same reason as every other grid
        // writer: the packed store's stencil is derived from the finished
        // grids.
        randomize::maze::writer::open_uninstalled_locks(rom, &state);
        randomize::maze::writer::stamp_pad_tiles(rom, &state);
        // And the same question from the fortress's end: which of the three
        // fortress tiles it wears says where the lock it opens is.
        if options.hints.hints_at_all() {
            randomize::maze::writer::stamp_fort_tiles(rom, &state);
        }
        rom.set_tag("wand_gate");
        randomize::wand_gate::apply(rom, wands);
        // Last of the grid writers, and the first thing that reads them: this
        // installs the packed store and the telepads themselves.
        rom.set_tag("world_persist");
        randomize::world_persist::apply(rom, &randomize::maze::writer::telepad_specs(&state));
        rom.set_tag("world_travel");
        randomize::world_travel::apply(rom);
    }

    // Every lock in the game, home and away, in one table — and with it the
    // rewritten fortress-FX effect that reads it. This is unconditional: the
    // effect replaces vanilla's outright, so a run that skipped it would leave
    // map operation 8 resolving slots out of tables this run has overwritten.
    //
    // It runs after the maze block because that is where the assignment is
    // decided when the mode is on. Away entries additionally need
    // `world_persist` to have installed the packed store already, since their
    // bit is looked up through it.
    //
    // **And after `world_order`, which is easy to miss.** A numbered lock shows
    // the world number the *player* sees, read from the display table that
    // module writes. Running before it would find the table unwritten, fall
    // through to the vanilla identity, and stamp numbers that appear nowhere in
    // the game — silently, since the tiles are still well-formed and the locks
    // still open. `lock_keys::the_digit_is_the_world_the_player_sees` asserts
    // the table is a permutation, which is what catches the wrong order.
    rom.set_tag("lock_keys");
    let lock_entries = maze_lock_keys.unwrap_or_else(|| lock_pairing.lock_entries(&build));
    randomize::lock_keys::apply(rom, &lock_entries, options.hints);

    // Big [?] bonus-room shuffle: every level with a Big [?] pipe draws from a
    // pool of 19 rooms (11 vanilla + 8 in the otherwise-dead "Unused Level 5").
    // Runs after the overworld builder because the BigQBlock_GotIt "already
    // opened" bit is per world *and* per screen, so the legal rooms for a host
    // depend on where its level ended up — and after `write_overworld`, so the
    // RNG it consumes cannot move the maps.
    // Off by default. `vanilla_assignments` takes no `rng` *by signature* and
    // writes no bytes, so with the option off every module downstream of here
    // sees the stream it would see if this pass did not exist. It just reports
    // where the eleven pipes already lead, so the 7-F1 protection below still
    // knows which room to protect.
    //
    // Note this is not whole-ROM identity with a pre-feature build: the lookup
    // routine `qol::fix_big_q_block_rooms` installs is unconditional and carries
    // the slot-seeding halves either way.
    let big_q_rooms = if options.shuffle_big_q_rooms {
        rom.set_tag("big_q_blocks/rooms");
        randomize::big_q_rooms::shuffle(rom, &mut rng)
    } else {
        randomize::big_q_rooms::vanilla_assignments()
    };

    // 7-F1 cannot be beaten without flight, so whatever room it drew has to
    // hand out a flight suit. Forced last, which is also why it needs no
    // cooperation from the contents roll above.
    if let Some(off) = big_q_rooms
        .iter()
        .find(|a| a.name == "7-F1")
        .and_then(|a| randomize::big_q_rooms::block_offset(rom, a.area, a.screen))
    {
        rom.set_tag("big_q_blocks/w7f1_flight");
        rom.write_byte(off, randomize::big_q_rooms::BIGQBLOCK_TANOOKI);
    }

    // Redraw + repack the ending credits mini-maps from the freshly-written
    // randomized world maps, then (if world order shuffled) reorder the montage
    // so each world's picture shows in the order the player traversed it. Both
    // run after `write_overworld` reads the final map tiles; the reorder must
    // run after the repack because it permutes the picture pointers.
    rom.set_tag("credits/world_maps");
    randomize::credits::render_world_maps(rom, &mut rng);
    if let Some(progression) = &credits_progression {
        rom.set_tag("credits/world_order");
        let order = randomize::credits::order_from_progression(progression);
        randomize::credits::reorder_world_pictures(rom, &order);
    }

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

    // The maze turns the whistle into fast travel between worlds already
    // visited, so `remove_whistles`' intent — "no skipping ahead" — is moot
    // here: a maze whistle can never reach anywhere new.
    //
    // **Forced ON in the maze, not off.** The mode grants a permanent whistle
    // of its own — `completion_bits`' new-game init writes one into inventory
    // slot 0, and it is never consumed — so a whistle in a chest, a Hammer Bro
    // drop or a Toad House is a duplicate of an item the player cannot run out
    // of: it occupies a slot and does nothing.
    //
    // This used to force the flag OFF, which put whistles *back* into the item
    // pool for the one mode with no use for them, and ignored the player's
    // setting in the process (it defaults to on).
    //
    // Note this flag has **no bearing on the maze's own whistle** — that comes
    // from the new-game init, not the item pool — and so none on the safety
    // property that whistle carries. See `world_travel` for that, and for what
    // would have to change if the mode ever shipped without one.
    let remove_whistles = options.remove_whistles || options.world_maze;
    if options.chest_items {
        rom.set_tag("items");
        randomize::items::randomize(rom, &mut rng, remove_whistles, piranha_active);
    } else if remove_whistles {
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

    // Randomize king quotes. Always called, even when the option is off: the
    // module draws its quotes unconditionally and only the ROM writes are
    // gated, so toggling this cannot shift the seed stream for anything below.
    rom.set_tag("king_quotes");
    randomize::king_quotes::randomize(rom, &mut rng, options.king_quotes);

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

    // Two-player "warp to partner" escape hatch (Start+Select on the map).
    // Always applied: in 2P the players share one map and its movable objects
    // (canoe, Hammer Bros), so one player can strand the other; this gives a
    // manual recovery. No effect in 1P (guarded on Total_Players).
    rom.set_tag("qol/map_warp");
    randomize::qol::apply_map_warp(rom);

    // Canoe "call the boat" rescue: press A on any dock to summon the shared
    // canoe to the adjacent water tile. Always applied — covers the canoe
    // softlocks map_warp can't (1P, and both players stranded). Works in any
    // world (keys on dock tile 0x4B + canoe object 0x10).
    rom.set_tag("qol/canoe_summon");
    randomize::qol::apply_canoe_summon(rom);

    // Stop an enemy that is jumping up at the player from turning a stomp into
    // damage. Always applied: it only widens outcomes vanilla already got
    // wrong, so there is nothing to opt out of.
    rom.set_tag("stomp_fairness");
    randomize::stomp_fairness::apply(rom);

    // One unit on the level clock is 41 frames in vanilla (~0.68 s), so the
    // displayed time runs ~47% fast. Always applied: the divider is simply the
    // wrong number, and a clock that reads seconds is what the status bar
    // already claims to show.
    rom.set_tag("qol/real_time_clock");
    randomize::qol::apply_real_time_clock(rom);

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

    // Bro encounters run on a 10-second clock instead of their header's time.
    if options.bro_battle_timer {
        rom.set_tag("qol/bro_battle_timer");
        randomize::qol::apply_bro_battle_timer(rom);
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
    // The maze forces it on: without it a game over wipes map completions, and
    // in a mode built on "a world you can come back to" that is the whole point
    // undone. It also makes the wipe uniform across all eight worlds, which
    // removes the "which half gets wiped" question from the packed store.
    if options.no_game_over_penalty || options.world_maze {
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
    //
    // The maze owns inventory slot 0 — its permanent whistle goes there in
    // `completion_bits`' new-game init — so the player's own items start at
    // slot 1 there. The inventory is a compacted list and the engine's panel
    // is dead while slot 0 is empty, so the two writers have to be contiguous
    // from the bottom; see `write_starting_items`.
    if !resolved_items.is_empty() {
        rom.set_tag("qol/starting_items");
        let first_slot = u8::from(options.world_maze);
        randomize::qol::write_starting_items(
            rom,
            seed,
            options.starting_lives,
            &resolved_items,
            first_slot,
        );
    }

    // MaCobra patches — always-on bugfixes and fairness tweaks.
    rom.set_tag("qol/macobra");
    randomize::qol::apply_macobra_patches(rom);

    // MaCobra52's "Remove Flashing" — suppress the palette-flash animation for
    // photosensitive-safe play. On by default; accessibility/cosmetic option,
    // not in the flag key and consumes no RNG.
    if options.remove_flashing {
        rom.set_tag("qol/remove_flashing");
        randomize::qol::apply_remove_flashing(rom);
    }

    // A defeated Lakitu is deleted instead of re-seeded two screens back, so it
    // stops holding one of the five general object slots for the whole level
    // (and stops feeding Spiny Eggs into the other four).
    if options.lakitu_stays_down {
        rom.set_tag("qol/lakitu_stays_down");
        randomize::qol::apply_lakitu_stays_down(rom);
    }

    // Faster Frog — speeds up Frog-Suit swimming. MUST run after
    // apply_macobra_patches: two of its writes patch inside the tail-swim
    // routine that macobra writes unconditionally, so it has to layer on top.
    if options.faster_frog {
        rom.set_tag("qol/faster_frog");
        randomize::qol::apply_faster_frog(rom);
    }

    // MaCobra52's "Easy Power-up System" — Small Mario gets suits / Fire power
    // without first growing Big (modern Mario power-up behavior).
    if options.modern_powerups {
        rom.set_tag("qol/modern_powerups");
        randomize::qol::apply_modern_powerups(rom);
    }

    // Random Fire Flower — in-level Fire Flower grants a position-derived suit
    // instead of always Fire. Pure static patch (no RNG): the substitution is a
    // deterministic function of World_Num + the flower's level position.
    if options.fire_flower != FireFlowerMode::Off {
        rom.set_tag("fire_flower");
        randomize::fire_flower::apply(rom, options.fire_flower);
    }

    // Poison Mushroom — each 1-Up block hands out either a real 1-Up or a
    // purple upside-down poison mushroom, chosen by a seed-salted position
    // hash. Installs the $0A trap object + a hook on the block-spawn sites.
    // Must run after world_order so the salt is final (mirrors fire_flower).
    // Replaces MaCobra52's all-1UPs-poison recolor under the same flag.
    if options.poison_mushrooms {
        rom.set_tag("poison_mushrooms");
        randomize::poison_mushroom::apply(rom);
    }

    // Stamp flag key + seed into free space at STAMP_OFFSET (PRG012):
    //   "S3R" magic, a length byte, the flag key bytes, then the seed
    //   (little-endian u64). The key bytes lead with their own format version,
    //   and since v29 their length varies with how much of the payload the
    //   options reach — hence the length byte, so the seed can still be found.
    rom.set_tag("stamp");
    let flag_bytes = options.to_flag_bytes();
    let mut stamp = Vec::with_capacity(4 + flag_bytes.len() + 8);
    stamp.extend_from_slice(b"S3R");
    stamp.push(flag_bytes.len() as u8);
    stamp.extend_from_slice(&flag_bytes);
    stamp.extend_from_slice(&seed.to_le_bytes());
    rom.write_range(STAMP_OFFSET, &stamp);
}
