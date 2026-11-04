//! The list of every level, for the modules that visit them all.

use std::collections::{BTreeMap, BTreeSet};

use crate::randomize::rom_data::{self, BETA_LEVELS, UNLISTED_LEVELS};
use crate::rom::Rom;

use super::{Area, PowerBlock, power_blocks, read_commands, trace};

/// Every level the game can load, each as its areas, main area first.
///
/// That is each world's pointer table, the beta stages (no vanilla pointer
/// reaches them, but `include_beta_stages` places them), and the levels the
/// engine loads by address (the Coin Ship). Entries that are not levels (the
/// start tile, pipes, bonus games) are skipped.
///
/// The same area can turn up in several levels: a level on two tiles, the
/// Hammer Bro arenas, a beta stage the builder has placed. Callers that write
/// dedupe by offset.
pub(crate) fn every_level(rom: &Rom) -> Vec<Vec<Area>> {
    let mut entries = Vec::new();
    for world in 0..8 {
        let tables = rom_data::world_tables(rom, world);
        entries.extend((0..tables.entry_count).map(|i| rom_data::read_entry(rom, &tables, i)));
    }
    entries.extend(BETA_LEVELS.iter().map(|b| b.entry()));
    entries.extend(UNLISTED_LEVELS.iter().map(|(_, e)| e.clone()));
    entries.iter().filter_map(|e| trace(rom, e).ok()).map(|(areas, _)| areas).collect()
}

/// Every powerup block in [`every_level`], once each, in file order.
pub(crate) fn every_power_block(rom: &Rom) -> Vec<PowerBlock> {
    let mut seen_areas = BTreeSet::new();
    let mut blocks = BTreeMap::new();
    for area in every_level(rom).iter().flatten() {
        if seen_areas.insert(area.header.offset) {
            for block in power_blocks(area, &read_commands(rom, area)) {
                blocks.insert(block.shape_offset(), block);
            }
        }
    }
    blocks.into_values().collect()
}
