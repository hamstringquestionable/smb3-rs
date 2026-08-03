//! Test-ROM builder: assemble a playtest ROM from orthogonal, composable knobs.
//!
//! This exists because playtesting needs ROMs that the randomizer proper would
//! never produce: a specific level parked on tile 1, a map with every lock
//! removed, a fortress reachable without clearing three levels first. Those
//! edits used to be a prose procedure re-derived by hand each time; here they
//! are code, driven by [`TestRomSpec`].
//!
//! The knobs are deliberately independent — what to place, what to walk
//! through, where to start, and whether the base map is vanilla or randomized
//! are four separate axes, so level testing, overworld testing, airship
//! testing, and lock testing are all combinations rather than named modes.

use crate::ips;
use crate::randomize::node_catalog::{EntryView, NodeCatalog};
use crate::randomize::rom_data::{self, LevelEntry, WORLDS};
use crate::randomize::world_order::WORLD_INIT_OPERAND;
use crate::rom::Rom;
use crate::{randomize_rom, Options};

/// Numbered level tiles: tile `0x03..=0x0F`, level number = tile - 2.
const NUMBERED_TILE_LO: u8 = 0x03;
const NUMBERED_TILE_HI: u8 = 0x0F;

/// File-offset window covering PRG010–011 of the ROM.
///
/// Practice-patch records outside this window rewrite PRG006/PRG012 (enemy
/// data and the overworld map) and would clobber whatever the randomizer just
/// produced, so the movement patch is applied as a subset over this range only.
const MOVEMENT_RECORD_RANGE: (usize, usize) = (0x14010, 0x18010);

/// Path tile to restore for each lock/gap tile.
///
/// Locks are a many-to-one encoding — `overworld_helpers::gap_tile_for` maps
/// every vertical path variant (plain, drawbridge, sky) onto a single lock
/// byte — so the inverse can only return the plain path tile of the right
/// orientation, not the original variant. In a test ROM that's a cosmetic
/// difference on a tile you're about to walk over.
const UNLOCK_TILES: &[(u8, u8)] = &[
    (0x54, 0x46), // vertical lock   → vertical path
    (0x56, 0x45), // horizontal lock → horizontal path
    (0xE4, 0xDA), // sky lock        → sky path
];

/// Water gaps are restored separately from locks — testing canoe/bridge
/// routing usually wants gaps intact even when locks are gone.
const UNGAP_TILES: &[(u8, u8)] = &[
    (0x9D, 0xB3), // water gap → bridge
];

/// Where the map and level layout come from before test edits are applied.
pub enum Base {
    /// Untouched vanilla ROM. The right base for testing a level's contents.
    Vanilla,
    /// Randomizer output. The right base for testing the overworld itself.
    Randomized { seed: u64, options: Box<Options> },
}

/// One `--place` request: a level name and where to park it.
pub struct Placement {
    /// Level name as the catalog spells it, e.g. `6F1`, `8B`, `7A`, `1-4`.
    pub level: String,
    /// Target numbered tile (1-based). `None` means "next free slot".
    pub slot: Option<u8>,
}

/// A full description of the test ROM to build.
pub struct TestRomSpec {
    pub base: Base,
    /// Levels to park on numbered tiles of the starting world.
    pub placements: Vec<Placement>,
    /// Replace *every* numbered level in the starting world with this one.
    pub place_all: Option<String>,
    /// Starting world, 1-based. `None` leaves the ROM's own starting world.
    pub world: Option<u8>,
    /// Practice-patch bytes supplying open map movement (walk over level,
    /// fortress and lock tiles without entering or clearing them).
    pub movement_patch: Option<Vec<u8>>,
    /// Replace lock tiles with walkable path.
    pub remove_locks: bool,
    /// Replace water-gap tiles with bridges.
    pub remove_gaps: bool,
    /// Item IDs to start with, up to 3 (the trampoline's slot count).
    pub starting_items: Vec<u8>,
    /// Starting lives. Only written when `starting_items` is non-empty, since
    /// the two share one trampoline.
    pub starting_lives: u8,
    /// Let the Hammer item break fortress lock tiles on the map.
    pub hammer_breaks_locks: bool,
    /// Let the Hammer item break water-gap (bridge) tiles on the map.
    pub hammer_breaks_bridges: bool,
    /// Include the 9 unreferenced beta stages as placeable names.
    pub include_beta: bool,
}

/// Result of a build: the ROM bytes plus a human-readable account of what was
/// changed, so the caller can report it without re-deriving anything.
#[derive(Debug)]
pub struct TestRom {
    pub bytes: Vec<u8>,
    pub report: Vec<String>,
}

/// Normalize a level name for matching: case-insensitive, dashes optional.
///
/// The catalog spells fortresses `6F1` while the CLI and docs have long used
/// `6-F1`; both must resolve, and neither spelling is worth making canonical.
fn norm(name: &str) -> String {
    name.chars()
        .filter(|c| *c != '-' && *c != '_' && !c.is_whitespace())
        .flat_map(|c| c.to_lowercase())
        .collect()
}

/// Catalog of the vanilla ROM, used to resolve source level names.
///
/// Always built from vanilla even when the base is randomized: `--place 6F1`
/// means "the level that is 6F1 in the original game", regardless of where the
/// randomizer has since moved it.
fn source_catalog(vanilla: &[u8], include_beta: bool) -> Result<Vec<EntryView>, String> {
    let rom = Rom::from_bytes_lax(vanilla, true).map_err(|e| e.to_string())?;
    Ok(NodeCatalog::build(&rom, include_beta).entry_views())
}

/// Resolve a level name to the data that travels with it when placed.
fn resolve(catalog: &[EntryView], name: &str) -> Result<LevelEntry, String> {
    let want = norm(name);
    let hit = catalog
        .iter()
        .find(|e| norm(&e.name) == want)
        .ok_or_else(|| format!("unknown level {name:?} (try --list)"))?;

    hit.level_entry.clone().ok_or_else(|| {
        format!("{name:?} is a {} and has no level data to place", hit.kind_label)
    })
}

/// Numbered-level slots in a world, ordered by level number.
///
/// Read from the *target* ROM so this works on a randomized base, where the
/// numbered tiles no longer sit where vanilla put them.
fn numbered_slots(rom: &Rom, world_idx: usize, include_beta: bool) -> Vec<(u8, usize)> {
    let mut slots: Vec<(u8, usize)> = NodeCatalog::build(rom, include_beta)
        .entry_views()
        .into_iter()
        .filter(|e| {
            e.world_idx == world_idx
                && e.is_numbered_level
                && (NUMBERED_TILE_LO..=NUMBERED_TILE_HI).contains(&e.tile)
        })
        .map(|e| (e.tile - 2, e.entry_idx))
        .collect();
    slots.sort_unstable();
    slots.dedup_by_key(|(num, _)| *num);
    slots
}

/// Rewrite lock and/or water-gap tiles across all 8 world maps.
fn open_map(rom: &mut Rom, remove_locks: bool, remove_gaps: bool) -> usize {
    let mut swaps: Vec<(u8, u8)> = Vec::new();
    if remove_locks {
        swaps.extend_from_slice(UNLOCK_TILES);
    }
    if remove_gaps {
        swaps.extend_from_slice(UNGAP_TILES);
    }
    if swaps.is_empty() {
        return 0;
    }

    let mut changed = 0;
    for world_idx in 0..8 {
        let grid = rom_data::read_tile_grid(rom, world_idx);
        for row in 0..grid.rows() {
            for col in 0..grid.cols {
                let tile = grid.get(row, col);
                if let Some(&(_, path)) = swaps.iter().find(|(lock, _)| *lock == tile) {
                    // Grids are stored screen-major (144 bytes per screen), so
                    // the offset is not `base + row * columns + col`.
                    rom.write_byte(rom_data::map_tile_offset(world_idx, row, col), path);
                    changed += 1;
                }
            }
        }
    }
    changed
}

/// Apply only the map-movement records of a practice patch.
fn apply_movement(rom: &mut Rom, patch: &[u8]) -> Result<(usize, usize), String> {
    let (start, end) = MOVEMENT_RECORD_RANGE;
    let rom_len = rom.output_bytes().len();

    let mut applied = 0;
    let mut written = 0;
    for rec in ips::parse_ips_records(patch)? {
        if !(start..end).contains(&rec.offset) {
            continue;
        }
        if rec.offset + rec.payload.len() > rom_len {
            return Err(format!(
                "movement record at 0x{:06X} (+{}) exceeds ROM size {rom_len}",
                rec.offset,
                rec.payload.len()
            ));
        }
        rom.write_range(rec.offset, &rec.payload);
        applied += 1;
        written += rec.payload.len();
    }
    Ok((applied, written))
}

/// Build a test ROM from vanilla bytes and a spec.
pub fn build(vanilla: &[u8], spec: &TestRomSpec) -> Result<TestRom, String> {
    let mut report = Vec::new();

    // 1. Base ROM — vanilla, or a full randomizer run.
    let base_bytes = match &spec.base {
        Base::Vanilla => {
            report.push("base: vanilla".to_string());
            vanilla.to_vec()
        }
        Base::Randomized { seed, options } => {
            report.push(format!("base: randomized (seed {seed})"));
            let rom = randomize_rom(vanilla, *seed, options, None)?;
            rom.output_bytes().to_vec()
        }
    };

    let mut rom = Rom::from_bytes_lax(&base_bytes, true).map_err(|e| e.to_string())?;

    // 2. Starting world. Resolved before placement so placements land in the
    //    world the player actually boots into.
    let world_idx = match spec.world {
        Some(w) => {
            let idx = (w as usize) - 1;
            rom.write_byte(WORLD_INIT_OPERAND, idx as u8);
            report.push(format!("start world: W{w}"));
            idx
        }
        None => rom.read_byte(WORLD_INIT_OPERAND) as usize,
    };
    if world_idx >= 8 {
        return Err(format!("starting world index {world_idx} out of range"));
    }

    // 3. Level placement.
    if spec.place_all.is_some() || !spec.placements.is_empty() {
        let catalog = source_catalog(vanilla, spec.include_beta)?;
        let slots = numbered_slots(&rom, world_idx, spec.include_beta);
        let world = &WORLDS[world_idx];

        if let Some(name) = &spec.place_all {
            let entry = resolve(&catalog, name)?;
            for (_, entry_idx) in &slots {
                rom_data::write_entry(&mut rom, world, *entry_idx, &entry);
            }
            report.push(format!(
                "placed {name} on all {} numbered tiles of W{}",
                slots.len(),
                world_idx + 1
            ));
        }

        let mut next_free = 0usize;
        for placement in &spec.placements {
            let entry = resolve(&catalog, &placement.level)?;
            let (num, entry_idx) = match placement.slot {
                Some(want) => *slots
                    .iter()
                    .find(|(num, _)| *num == want)
                    .ok_or_else(|| {
                        format!(
                            "W{} has no numbered tile {want} (available: {})",
                            world_idx + 1,
                            slots
                                .iter()
                                .map(|(n, _)| n.to_string())
                                .collect::<Vec<_>>()
                                .join(", ")
                        )
                    })?,
                None => {
                    let slot = *slots.get(next_free).ok_or_else(|| {
                        format!(
                            "W{} has only {} numbered tiles; too many --place entries",
                            world_idx + 1,
                            slots.len()
                        )
                    })?;
                    next_free += 1;
                    slot
                }
            };
            rom_data::write_entry(&mut rom, world, entry_idx, &entry);
            report.push(format!(
                "placed {} on W{}-{num}",
                placement.level,
                world_idx + 1
            ));
        }
    }

    // 4. Open the map up.
    let opened = open_map(&mut rom, spec.remove_locks, spec.remove_gaps);
    if opened > 0 {
        let what = match (spec.remove_locks, spec.remove_gaps) {
            (true, true) => "locks + water gaps",
            (true, false) => "locks",
            _ => "water gaps",
        };
        report.push(format!("removed {opened} {what} across all 8 maps"));
    }
    if !spec.remove_locks {
        report.push("locks: kept".to_string());
    }

    // 5. Open movement (walk over level/fortress/lock tiles).
    match &spec.movement_patch {
        Some(patch) => {
            let (applied, written) = apply_movement(&mut rom, patch)?;
            report.push(format!(
                "open movement: {applied} records ({written} bytes)"
            ));
        }
        None => report.push("open movement: off".to_string()),
    }

    // 6. Hammer tile-breaking. Applied here rather than via `Options` so it
    //    works on a vanilla base too — testing what a hammer does to a lock
    //    shouldn't require randomizing the map first.
    if spec.hammer_breaks_locks || spec.hammer_breaks_bridges {
        crate::randomize::qol::hammer_breaks_tiles(
            &mut rom,
            spec.hammer_breaks_locks,
            spec.hammer_breaks_bridges,
        );
        let what = match (spec.hammer_breaks_locks, spec.hammer_breaks_bridges) {
            (true, true) => "locks + bridges",
            (true, false) => "locks",
            _ => "bridges",
        };
        report.push(format!("hammer breaks: {what}"));
    }

    // 7. Starting inventory. Last, mirroring the randomizer's own ordering —
    //    the trampoline overwrites title-screen bytes and must win.
    if !spec.starting_items.is_empty() {
        let items: Vec<u8> = spec.starting_items.iter().copied().take(3).collect();
        // Seed only drives the intro-skip menu music; any fixed value is fine
        // for a test ROM and keeps output reproducible.
        crate::randomize::qol::write_starting_items(&mut rom, 0, spec.starting_lives, &items);
        report.push(format!(
            "inventory: {} ({} lives)",
            items
                .iter()
                .map(|&id| crate::item_display_name(id))
                .collect::<Vec<_>>()
                .join(", "),
            spec.starting_lives
        ));
        if spec.starting_items.len() > 3 {
            report.push(format!(
                "  note: {} extra item(s) dropped — only 3 inventory slots exist",
                spec.starting_items.len() - 3
            ));
        }
    }

    Ok(TestRom { bytes: rom.output_bytes().to_vec(), report })
}

/// All placeable level names, for `--list`.
pub fn list_levels(vanilla: &[u8], include_beta: bool) -> Result<Vec<String>, String> {
    let catalog = source_catalog(vanilla, include_beta)?;
    Ok(catalog
        .iter()
        .filter(|e| e.level_entry.is_some())
        .map(|e| format!("{:<8} {}", e.name, e.kind_label))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    const VANILLA: &str = "roms/Super Mario Bros. 3 (USA) (Rev 1).nes";

    fn vanilla() -> Option<Vec<u8>> {
        std::fs::read(VANILLA).ok()
    }

    fn spec() -> TestRomSpec {
        TestRomSpec {
            base: Base::Vanilla,
            placements: Vec::new(),
            place_all: None,
            world: None,
            movement_patch: None,
            remove_locks: false,
            remove_gaps: false,
            starting_items: Vec::new(),
            starting_lives: 5,
            hammer_breaks_locks: false,
            hammer_breaks_bridges: false,
            include_beta: false,
        }
    }

    #[test]
    fn name_normalization_accepts_both_spellings() {
        assert_eq!(norm("6-F1"), norm("6f1"));
        assert_eq!(norm("8B"), norm("8b"));
        assert_eq!(norm("8-Tank"), norm("8tank"));
    }

    #[test]
    fn placement_writes_the_source_levels_pointers() {
        let Some(van) = vanilla() else { return };

        // 6F1 is W6's first fortress: FORTRESS_ENTRIES lists it at (5, 9).
        let src = Rom::from_bytes_lax(&van, true).unwrap();
        let want = rom_data::read_entry(&src, &WORLDS[5], 9);

        let built = build(
            &van,
            &TestRomSpec {
                placements: vec![Placement { level: "6-F1".into(), slot: Some(1) }],
                world: Some(1),
                ..spec()
            },
        )
        .unwrap();

        let out = Rom::from_bytes_lax(&built.bytes, true).unwrap();
        let slot = numbered_slots(&out, 0, false)
            .into_iter()
            .find(|(num, _)| *num == 1)
            .expect("W1 has a tile 1");
        assert_eq!(rom_data::read_entry(&out, &WORLDS[0], slot.1), want);
    }

    /// Regression: map grids are screen-major, so a row-major offset walks off
    /// into the wrong screen and corrupts unrelated tiles. Unlocking must leave
    /// zero lock bytes behind and must not touch anything else.
    #[test]
    fn unlocking_clears_every_lock_and_nothing_else() {
        let Some(van) = vanilla() else { return };

        let built = build(&van, &TestRomSpec { remove_locks: true, ..spec() }).unwrap();
        let out = Rom::from_bytes_lax(&built.bytes, true).unwrap();

        let locks: Vec<u8> = UNLOCK_TILES.iter().map(|(l, _)| *l).collect();
        for world_idx in 0..8 {
            let grid = rom_data::read_tile_grid(&out, world_idx);
            for row in 0..grid.rows() {
                for col in 0..grid.cols {
                    assert!(
                        !locks.contains(&grid.get(row, col)),
                        "lock survived at W{} ({row},{col})",
                        world_idx + 1
                    );
                }
            }
        }

        // Every changed byte must be a lock→path swap, nothing collateral.
        for (i, (before, after)) in van.iter().zip(built.bytes.iter()).enumerate() {
            if before != after {
                assert!(
                    UNLOCK_TILES.contains(&(*before, *after)),
                    "unexpected write at 0x{i:05X}: {before:#04X} -> {after:#04X}"
                );
            }
        }
    }

    #[test]
    fn gaps_and_locks_are_independent_knobs() {
        let Some(van) = vanilla() else { return };

        let locks_only = build(&van, &TestRomSpec { remove_locks: true, ..spec() }).unwrap();
        for (i, (before, after)) in van.iter().zip(locks_only.bytes.iter()).enumerate() {
            assert!(
                before == after || *before != 0x9D,
                "water gap at 0x{i:05X} cleared despite remove_gaps = false"
            );
        }

        let gaps_only = build(&van, &TestRomSpec { remove_gaps: true, ..spec() }).unwrap();
        for (i, (before, after)) in van.iter().zip(gaps_only.bytes.iter()).enumerate() {
            assert!(
                before == after || *before == 0x9D,
                "non-gap byte changed at 0x{i:05X} despite remove_locks = false"
            );
        }
    }

    #[test]
    fn starting_world_is_written() {
        let Some(van) = vanilla() else { return };
        let built = build(&van, &TestRomSpec { world: Some(7), ..spec() }).unwrap();
        assert_eq!(built.bytes[WORLD_INIT_OPERAND], 6);
    }

    /// The inventory bytes must actually reach the trampoline, and asking for
    /// items must not disturb the map.
    #[test]
    fn starting_items_are_written_and_touch_nothing_on_the_map() {
        let Some(van) = vanilla() else { return };

        let built = build(
            &van,
            &TestRomSpec {
                starting_items: vec![0x0B, 0x03, 0x02], // hammer, leaf, fire
                starting_lives: 25,
                ..spec()
            },
        )
        .unwrap();

        // Each item is written by an `LDA #item / STA $7D80+i` pair.
        for (i, item) in [0x0Bu8, 0x03, 0x02].into_iter().enumerate() {
            let pair = [0xA9, item, 0x8D, 0x80 + i as u8, 0x7D];
            assert!(
                built.bytes.windows(5).any(|w| w == pair),
                "no write found for item {item:#04X} in slot {i}"
            );
        }
        assert!(built.bytes.windows(2).any(|w| w == [0xA9, 25]), "lives not written");

        for world_idx in 0..8 {
            let before = Rom::from_bytes_lax(&van, true).unwrap();
            let after = Rom::from_bytes_lax(&built.bytes, true).unwrap();
            assert_eq!(
                rom_data::read_tile_grid(&before, world_idx).tiles,
                rom_data::read_tile_grid(&after, world_idx).tiles,
                "starting items changed W{}'s map",
                world_idx + 1
            );
        }
    }

    /// Locks must survive `--keep-locks` even when the hammer can break them —
    /// that combination is the whole point of lock-FX testing.
    #[test]
    fn hammer_breaks_locks_leaves_the_locks_in_place() {
        let Some(van) = vanilla() else { return };

        let built = build(
            &van,
            &TestRomSpec { hammer_breaks_locks: true, remove_locks: false, ..spec() },
        )
        .unwrap();

        let out = Rom::from_bytes_lax(&built.bytes, true).unwrap();
        let locks: Vec<u8> = UNLOCK_TILES.iter().map(|(l, _)| *l).collect();
        let surviving: usize = (0..8)
            .map(|w| {
                let grid = rom_data::read_tile_grid(&out, w);
                (0..grid.rows())
                    .flat_map(|r| (0..grid.cols).map(move |c| (r, c)))
                    .filter(|&(r, c)| locks.contains(&grid.get(r, c)))
                    .count()
            })
            .sum();
        assert!(surviving > 0, "hammer patch removed the locks it should break at runtime");
    }

    #[test]
    fn unknown_level_name_is_an_error() {
        let Some(van) = vanilla() else { return };
        let err = build(
            &van,
            &TestRomSpec {
                placements: vec![Placement { level: "9-9".into(), slot: None }],
                ..spec()
            },
        )
        .unwrap_err();
        assert!(err.contains("unknown level"), "got: {err}");
    }

    /// A hammer bro / toad house has no placeable level data; the failure
    /// should name the kind rather than silently writing garbage pointers.
    #[test]
    fn placing_a_non_level_entry_explains_why() {
        let Some(van) = vanilla() else { return };
        let err = build(
            &van,
            &TestRomSpec {
                placements: vec![Placement { level: "1S".into(), slot: None }],
                ..spec()
            },
        )
        .unwrap_err();
        assert!(err.contains("start") || err.contains("unknown"), "got: {err}");
    }
}
