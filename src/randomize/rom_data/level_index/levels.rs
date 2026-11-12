//! The list of every level, for the modules that visit them all.

use std::collections::{BTreeMap, BTreeSet};

use crate::randomize::rom_data::{self, BETA_LEVELS, UNLISTED_LEVELS};
use crate::rom::Rom;

use super::{Area, LEVEL_HEADER_SIZE, PowerBlock, power_blocks, read_commands, trace};

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

/// The headers in `levels` whose nine bytes are also another area's bytes
/// (its header or its commands). A writer must leave these alone: a byte two
/// areas read cannot be changed for one without changing the other.
///
/// Vanilla has one such pair. 3HB18's header (`0x213F7`) sits inside 7-3's
/// last command, and its byte 8 is byte 4 of the W1 Hammer Bro arena header
/// after it (`0x213FB`) — the arena's width. Both headers are returned, so
/// the answer does not depend on which levels a seed places: without 7-3 on
/// a map, nothing else marks 3HB18 as data rather than a header.
pub(crate) fn shared_headers(levels: &[Vec<Area>]) -> BTreeSet<usize> {
    let spans: BTreeSet<(usize, usize)> =
        levels.iter().flatten().map(|a| (a.header.offset, a.terminator)).collect();
    spans
        .iter()
        .map(|&(h, _)| h)
        .filter(|&h| {
            let last = h + LEVEL_HEADER_SIZE - 1;
            spans.iter().any(|&(s, e)| s != h && s <= last && h <= e)
        })
        .collect()
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
