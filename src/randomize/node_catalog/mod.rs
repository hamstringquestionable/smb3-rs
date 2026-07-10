//! Static node catalog: classifies every pointer table entry across all 8 worlds.
//!
//! This is a read-only data extraction pass — no RNG, no ROM writes. It produces
//! a `NodeCatalog` containing a `CatalogEntry` for each of the 340 entries in the
//! game, with classification, human-readable name, grid position, and level data.
//!
//! Used by the overworld builder pipeline as the source of truth for what exists
//! on each map before any shuffling occurs.

use crate::rom::Rom;

use super::rom_data::{self, BETA_LEVELS, HB_EXCLUDE_OBJ_PTRS, LevelEntry, Pos};

mod classify;
mod naming;
mod pipes;

use classify::classify_world;
use naming::assign_names;

/// Raw fields produced for each pointer table entry by `classify_world`,
/// before they're packaged into a full `CatalogEntry`.
pub(super) type RawClassifiedEntry = (usize, NodeKind, Pos, u8, Option<LevelEntry>);

// ---------------------------------------------------------------------------
// Node types
// ---------------------------------------------------------------------------

/// Classification of a pointer table entry.
#[derive(Clone, Debug)]
pub(super) enum NodeKind {
    /// Numbered action level.
    Level,
    /// Fortress (carries Boom-Boom Y-byte offset for patching).
    Fortress { boomboom_y_offset: usize },
    /// One endpoint of a pipe pair.
    /// `is_a_side`: true if this is the A endpoint (upper nibble in dest tables).
    Pipe { dest_idx: usize, is_a_side: bool },
    /// Airship dock.
    Airship,
    /// Bowser's castle (W8 only).
    Bowser,
    /// Start tile — fixed position, never moves.
    Start,
    /// Toad house.
    ToadHouse,
    /// Bonus / card matching game.
    BonusGame,
    /// Hammer Brother encounter.
    HammerBro,
    /// Entry linked to an overworld map object sprite (e.g., W7 piranha plants).
    MapObject,
}

impl NodeKind {
    /// Whether this node is a placeable game level (enters the shuffle pool).
    pub fn is_level_like(&self) -> bool {
        matches!(
            self,
            NodeKind::Level
                | NodeKind::Fortress { .. }
                | NodeKind::Pipe { .. }
                | NodeKind::Airship
                | NodeKind::Bowser
        )
    }
}

// ---------------------------------------------------------------------------
// Catalog entry
// ---------------------------------------------------------------------------

/// A single classified pointer table entry.
#[derive(Clone, Debug)]
pub(super) struct CatalogEntry {
    pub world_idx: usize,
    pub entry_idx: usize,
    pub kind: NodeKind,
    /// Human-readable name (e.g., "1-1", "3F2", "7-P1", "8B").
    pub name: String,
    /// Vanilla grid position (row, col).
    pub grid_pos: (usize, usize),
    /// Vanilla map tile at this position.
    pub tile: u8,
    /// Level entry data (tileset, obj/lay ptrs). None for Start.
    pub level_entry: Option<LevelEntry>,
}

// ---------------------------------------------------------------------------
// Node catalog
// ---------------------------------------------------------------------------

/// Complete catalog of all pointer table entries across all 8 worlds.
pub(crate) struct NodeCatalog {
    pub(super) entries: Vec<CatalogEntry>,
    /// Per-world flag: when true, the Start and Airship entries' `grid_pos`
    /// have been swapped (Mario spawns at the airship coords, the airship/
    /// objective lives at the start coords). Index 7 (W8) is always false —
    /// Bowser's castle has no slot-1 airship sprite to move.
    pub(super) start_airship_swapped: [bool; 8],
}

impl NodeCatalog {
    /// Build the catalog by reading and classifying every entry from the ROM.
    ///
    /// When `include_beta_stages` is true, synthetic entries for the 9
    /// unreferenced beta levels are appended after the 340 vanilla entries.
    /// They use `world_idx = usize::MAX` and `entry_idx = usize::MAX` as
    /// sentinels (no vanilla pointer table home).
    pub(crate) fn build(rom: &Rom, include_beta_stages: bool) -> Self {
        let mut entries = Vec::with_capacity(340 + if include_beta_stages { BETA_LEVELS.len() } else { 0 });

        // First pass: classify all entries (names assigned in second pass)
        for wi in 0..8 {
            let grid = rom_data::read_tile_grid(rom, wi);
            let world_entries = classify_world(rom, wi, &grid);

            for (entry_idx, kind, grid_pos, tile, level_entry) in world_entries {
                entries.push(CatalogEntry {
                    world_idx: wi,
                    entry_idx,
                    kind,
                    name: String::new(), // filled in second pass
                    grid_pos,
                    tile,
                    level_entry,
                });
            }
        }

        // Append synthetic beta level entries (no vanilla home).
        if include_beta_stages {
            for beta in BETA_LEVELS {
                entries.push(CatalogEntry {
                    world_idx: usize::MAX,
                    entry_idx: usize::MAX,
                    kind: NodeKind::Level,
                    name: beta.name.to_string(),
                    grid_pos: (usize::MAX, usize::MAX),
                    tile: 0,
                    level_entry: Some(LevelEntry {
                        tileset: beta.tileset,
                        obj_lo: beta.obj_lo,
                        obj_hi: beta.obj_hi,
                        lay_lo: beta.lay_lo,
                        lay_hi: beta.lay_hi,
                    }),
                });
            }
        }

        // Second pass: assign names (beta entries already have names set)
        assign_names(&mut entries);

        NodeCatalog { entries, start_airship_swapped: [false; 8] }
    }

    /// Reclassify map-object-linked entries (the W7 piranha plant levels) as
    /// plain levels so they enter the shuffle pool. Called when piranha
    /// shuffle is active; pairs with `piranha_rooms::clear_vanilla_plants`,
    /// which frees their sprites/positions on the ROM side.
    pub(crate) fn release_map_objects(&mut self) {
        for e in &mut self.entries {
            if matches!(e.kind, NodeKind::MapObject) {
                e.kind = NodeKind::Level;
            }
        }
    }

    /// Iterate entries for a specific world.
    pub(super) fn world(&self, world_idx: usize) -> impl Iterator<Item = &CatalogEntry> {
        self.entries.iter().filter(move |e| e.world_idx == world_idx)
    }

    /// Collect unique real HammerBro levels (obj >= 0xC000).
    /// Excludes toad house / bonus game pointer formats.
    pub(super) fn unique_hammer_bro_levels(&self) -> Vec<rom_data::LevelEntry> {
        let mut seen = std::collections::HashSet::new();
        let mut result = Vec::new();
        for e in &self.entries {
            if !matches!(e.kind, NodeKind::HammerBro) {
                continue;
            }
            if let Some(le) = &e.level_entry {
                let obj = (le.obj_hi as u16) << 8 | le.obj_lo as u16;
                if obj >= 0xC000
                    && !HB_EXCLUDE_OBJ_PTRS.contains(&obj)
                    && !rom_data::HB_EXCLUDE_ENTRIES.contains(&(obj, le.tileset))
                    && seen.insert(le.clone())
                {
                    result.push(le.clone());
                }
            }
        }
        result
    }
}

#[cfg(test)]
mod tests;
