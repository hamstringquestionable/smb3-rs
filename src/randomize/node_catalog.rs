/// Static node catalog: classifies every pointer table entry across all 8 worlds.
///
/// This is a read-only data extraction pass — no RNG, no ROM writes. It produces
/// a `NodeCatalog` containing a `CatalogEntry` for each of the 340 entries in the
/// game, with classification, human-readable name, grid position, and level data.
///
/// Used by the overworld builder pipeline as the source of truth for what exists
/// on each map before any shuffling occurs.

use std::collections::{HashMap, HashSet};

use crate::rom::Rom;

use super::rom_data::{
    self, AIRSHIP_ENTRIES, BETA_LEVELS, BOWSER_ENTRY, FORTRESS_ENTRIES,
    HAMMER_BRO_OBJ_PTRS, HB_EXCLUDE_OBJ_PTRS, LevelEntry, MAP_OBJ_ENTRY_LINKS,
    TOAD_HOUSE_OBJ_PTRS, PIPE_MAP_X, PIPE_MAP_XHI, PIPE_MAP_Y, TILE_START, WORLDS,
};

// ---------------------------------------------------------------------------
// W5 Spiral Tower entries (functionally a pipe pair using dest index 0)
// ---------------------------------------------------------------------------

const W5_SPIRAL_ENTRIES: &[(usize, usize)] = &[(4, 10), (4, 21)];

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

        NodeCatalog { entries }
    }

    /// Iterate entries for a specific world.
    #[allow(dead_code)] // used in tests
    pub(super) fn world(&self, world_idx: usize) -> impl Iterator<Item = &CatalogEntry> {
        self.entries.iter().filter(move |e| e.world_idx == world_idx)
    }

    /// Iterate entries matching a kind predicate.
    #[allow(dead_code)] // used in tests
    pub(super) fn by_kind(&self, pred: fn(&NodeKind) -> bool) -> impl Iterator<Item = &CatalogEntry> {
        self.entries.iter().filter(move |e| pred(&e.kind))
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

// ---------------------------------------------------------------------------
// Per-world classification
// ---------------------------------------------------------------------------

/// Classify all entries in a single world.
/// Returns: Vec of (entry_idx, kind, grid_pos, tile, level_entry).
fn classify_world(
    rom: &Rom,
    world_idx: usize,
    grid: &rom_data::Grid,
) -> Vec<(usize, NodeKind, (usize, usize), u8, Option<LevelEntry>)> {
    let world = &WORLDS[world_idx];
    let n = world.entry_count;
    let (_scrcol, objsets, layouts) = rom_data::table_offsets(world);

    // -- Pre-compute sets for classification --

    // Map-object-linked entries
    let map_obj_entries: HashSet<usize> = MAP_OBJ_ENTRY_LINKS
        .iter()
        .filter(|&&(w, _, _)| w == world_idx)
        .map(|&(_, _, entry_idx)| entry_idx)
        .collect();

    // Pipe detection: PIPEWAYCONTROLLER (0x25) enemy, grouped by obj_ptr
    let mut pipe_entries_by_obj: HashMap<u16, Vec<usize>> = HashMap::new();
    let mut spiral_entries: Vec<usize> = Vec::new();
    for i in 0..n {
        let obj = rom_data::read_word(rom, objsets + i * 2);
        if W5_SPIRAL_ENTRIES.contains(&(world_idx, i)) {
            spiral_entries.push(i);
        } else if rom_data::has_enemy_id(rom, obj, 0x25) {
            pipe_entries_by_obj.entry(obj).or_default().push(i);
        }
    }

    // Build pipe pair map: entry_idx → dest_idx
    let dest_indices = rom_data::dest_indices_for_world(world_idx);
    let pipe_map = build_pipe_map(rom, world_idx, &pipe_entries_by_obj, &spiral_entries, &dest_indices);

    // -- Classify each entry --
    let mut result = Vec::with_capacity(n);

    for i in 0..n {
        let (row, col) = rom_data::entry_grid_position(rom, world, i);
        let obj = rom_data::read_word(rom, objsets + i * 2);
        let lay = rom_data::read_word(rom, layouts + i * 2);
        let map_tile = if row < grid.rows && col < grid.cols {
            grid.get(row, col)
        } else {
            0xFF
        };

        let kind = classify_entry(
            rom, world_idx, i, obj, lay, map_tile, row,
            &map_obj_entries, &pipe_map,
        );

        let level_entry = if matches!(kind, NodeKind::Start) {
            None
        } else {
            Some(rom_data::read_entry(rom, world, i))
        };

        result.push((i, kind, (row, col), map_tile, level_entry));
    }

    result
}

/// Classify a single entry into a NodeKind.
fn classify_entry(
    rom: &Rom,
    world_idx: usize,
    entry_idx: usize,
    obj: u16,
    lay: u16,
    map_tile: u8,
    row: usize,
    map_obj_entries: &HashSet<usize>,
    pipe_map: &HashMap<usize, (usize, bool)>,
) -> NodeKind {
    // 1. Start tile
    if map_tile == TILE_START {
        return NodeKind::Start;
    }

    // 2. Bowser's castle
    if (world_idx, entry_idx) == BOWSER_ENTRY {
        return NodeKind::Bowser;
    }

    // 3. Airship
    if AIRSHIP_ENTRIES.contains(&(world_idx, entry_idx)) {
        return NodeKind::Airship;
    }

    // 4. Fortress
    if FORTRESS_ENTRIES.contains(&(world_idx, entry_idx)) {
        let entry = rom_data::read_entry(rom, &WORLDS[world_idx], entry_idx);
        let obj_ptr = (entry.obj_hi as u16) << 8 | entry.obj_lo as u16;
        let boomboom_y_offset = rom_data::boomboom_y_offset_for_obj(obj_ptr).unwrap_or(0);
        return NodeKind::Fortress { boomboom_y_offset };
    }

    // 5. Pipe (PIPEWAYCONTROLLER or W5 spiral)
    if let Some(&(dest_idx, is_a_side)) = pipe_map.get(&entry_idx) {
        return NodeKind::Pipe { dest_idx, is_a_side };
    }

    // 6. Toad house (standard $0700 + variant reward formats)
    if TOAD_HOUSE_OBJ_PTRS.contains(&obj) {
        return NodeKind::ToadHouse;
    }

    // 7. Bonus game
    if obj == 0x0001 && lay == 0x0000 {
        return NodeKind::BonusGame;
    }

    // 8. Map object (W7 piranha plants, etc.)
    if map_obj_entries.contains(&entry_idx) {
        return NodeKind::MapObject;
    }

    // 9. Hammer bro (known hammer bro obj_ptrs)
    if HAMMER_BRO_OBJ_PTRS.contains(&obj) {
        return NodeKind::HammerBro;
    }

    // 10. Non-level entries (out of bounds, special pointers)
    if row >= rom_data::ROWS || !rom_data::is_level_pointer(obj, lay) {
        return NodeKind::HammerBro;
    }

    // 11. Regular action level
    NodeKind::Level
}

// ---------------------------------------------------------------------------
// Pipe pair matching
// ---------------------------------------------------------------------------

/// Build a map from entry_idx → (dest_idx, is_a_side) for all pipe entries.
///
/// The A-side is the entry whose dest table upper nibble encodes its position.
/// For regular pipe pairs (both share an obj_ptr and have PIPEWAYCONTROLLER),
/// A-side has layout byte5 bit 6 = 0.  For mixed pairs (one has PWC, one
/// doesn't — e.g. W5 spiral castle), the PWC entry is the A-side.
fn build_pipe_map(
    rom: &Rom,
    world_idx: usize,
    pipe_entries_by_obj: &HashMap<u16, Vec<usize>>,
    spiral_entries: &[usize],
    dest_indices: &[usize],
) -> HashMap<usize, (usize, bool)> {
    let world = &WORLDS[world_idx];
    let mut result: HashMap<usize, (usize, bool)> = HashMap::new();

    // Collect all pipe pairs: (entry_a, entry_b)
    let mut pairs: Vec<(usize, usize)> = Vec::new();

    // Regular pipe pairs: grouped by obj_ptr
    let mut keys: Vec<u16> = pipe_entries_by_obj.keys().copied().collect();
    keys.sort();
    for key in keys {
        let group = &pipe_entries_by_obj[&key];
        if group.len() == 2 {
            pairs.push((group[0], group[1]));
        }
    }

    // W5 spiral tower pair
    if world_idx == 4 && spiral_entries.len() == 2 {
        let mut sorted = spiral_entries.to_vec();
        sorted.sort();
        pairs.push((sorted[0], sorted[1]));
    }

    // Match pairs to dest indices by comparing grid positions
    for &(ea, eb) in &pairs {
        let ea_pos = rom_data::entry_grid_position(rom, world, ea);
        let eb_pos = rom_data::entry_grid_position(rom, world, eb);

        for &d in dest_indices {
            let (da, db) = read_dest_positions(rom, d);
            if (ea_pos == da && eb_pos == db) || (ea_pos == db && eb_pos == da) {
                let (a_entry, b_entry) = classify_pipe_ab(rom, world, ea, eb);
                result.insert(a_entry, (d, true));
                result.insert(b_entry, (d, false));
                break;
            }
        }
    }

    result
}

/// Determine which of two pipe entries is the A-side (upper nibble in dest tables).
///
/// Mixed pairs (one has PIPEWAYCONTROLLER, one doesn't): PWC entry → A-side.
/// Regular pairs (both have PWC): layout byte5 bit 6 = 0 → A-side.
/// Fallback: (ea, eb) order preserved.
fn classify_pipe_ab(rom: &Rom, world: &rom_data::WorldTables, ea: usize, eb: usize) -> (usize, usize) {
    let le_a = rom_data::read_entry(rom, world, ea);
    let le_b = rom_data::read_entry(rom, world, eb);

    let obj_a = u16::from_le_bytes([le_a.obj_lo, le_a.obj_hi]);
    let obj_b = u16::from_le_bytes([le_b.obj_lo, le_b.obj_hi]);

    let has_pwc_a = rom_data::has_enemy_id(rom, obj_a, 0x25);
    let has_pwc_b = rom_data::has_enemy_id(rom, obj_b, 0x25);

    // Mixed pair: PWC entry is A-side.
    if has_pwc_a && !has_pwc_b {
        return (ea, eb);
    }
    if has_pwc_b && !has_pwc_a {
        return (eb, ea);
    }

    // Regular pair: use layout byte5 bit 6. A-side has bit 6 = 0.
    let lay_ptr_a = u16::from_le_bytes([le_a.lay_lo, le_a.lay_hi]);
    if let Some(file_off) = rom_data::layout_file_offset(lay_ptr_a, le_a.tileset) {
        let byte5 = rom.read_byte(file_off + 5);
        if byte5 & 0x40 == 0 {
            return (ea, eb);
        } else {
            return (eb, ea);
        }
    }

    // Fallback: preserve original order.
    (ea, eb)
}

/// Read the A and B endpoint positions from the pipe destination tables.
fn read_dest_positions(rom: &Rom, dest_idx: usize) -> ((usize, usize), (usize, usize)) {
    let xhi = rom.read_byte(PIPE_MAP_XHI + dest_idx);
    let x = rom.read_byte(PIPE_MAP_X + dest_idx);
    let y = rom.read_byte(PIPE_MAP_Y + dest_idx);

    let a_pos = (
        ((y >> 4) as usize).wrapping_sub(2),
        ((xhi >> 4) as usize) * 16 + ((x >> 4) as usize),
    );
    let b_pos = (
        ((y & 0xF) as usize).wrapping_sub(2),
        ((xhi & 0xF) as usize) * 16 + ((x & 0xF) as usize),
    );
    (a_pos, b_pos)
}

// ---------------------------------------------------------------------------
// Naming
// ---------------------------------------------------------------------------

/// Special name overrides for entries that sit on non-standard tiles.
const LEVEL_NAME_OVERRIDES: &[(usize, usize, &str)] = &[
    (1, 32, "2-QS"),     // quicksand
    (1, 42, "2-Pyr"),    // pyramid
    (4, 10, "5-SC"),     // spiral castle
    (6, 11, "7-P1"),     // piranha plant 1
    (6, 45, "7-P2"),     // piranha plant 2
    (7, 5, "8-Tank"),    // tank level
    (7, 7, "8-Navy"),    // battleship
    (7, 10, "8-Air"),    // air force
    (7, 14, "8-Hnd1"),   // hand trap 1
    (7, 15, "8-Hnd2"),   // hand trap 2
    (7, 16, "8-Hnd3"),   // hand trap 3
    (7, 36, "8-STnk"),   // super tank
];

/// Assign human-readable names to all catalog entries.
///
/// Two-pass: first count per-world per-kind totals, then assign names
/// with ordinal suffixes when a world has multiples of the same kind.
fn assign_names(entries: &mut [CatalogEntry]) {
    // Count per-world kind totals for ordinal suffixes
    let mut fortress_counts: [usize; 8] = [0; 8];
    let mut toad_counts: [usize; 8] = [0; 8];
    let mut bonus_counts: [usize; 8] = [0; 8];
    let mut hammer_counts: [usize; 8] = [0; 8];
    let mut map_obj_counts: [usize; 8] = [0; 8];
    let mut pipe_counts: [usize; 8] = [0; 8];

    for e in entries.iter() {
        if e.world_idx == usize::MAX { continue; } // skip synthetic (beta) entries
        match e.kind {
            NodeKind::Fortress { .. } => fortress_counts[e.world_idx] += 1,
            NodeKind::ToadHouse => toad_counts[e.world_idx] += 1,
            NodeKind::BonusGame => bonus_counts[e.world_idx] += 1,
            NodeKind::HammerBro => hammer_counts[e.world_idx] += 1,
            NodeKind::MapObject => map_obj_counts[e.world_idx] += 1,
            NodeKind::Pipe { .. } => pipe_counts[e.world_idx] += 1,
            _ => {}
        }
    }

    // Track ordinals per world per kind
    let mut fortress_ord: [usize; 8] = [0; 8];
    let mut toad_ord: [usize; 8] = [0; 8];
    let mut bonus_ord: [usize; 8] = [0; 8];
    let mut hammer_ord: [usize; 8] = [0; 8];
    let mut map_obj_ord: [usize; 8] = [0; 8];
    let mut pipe_ord: [usize; 8] = [0; 8];

    for e in entries.iter_mut() {
        // Skip synthetic entries (betas) — they already have names.
        if e.world_idx == usize::MAX {
            continue;
        }
        let w = e.world_idx;
        let w1 = w + 1; // 1-indexed for display

        // Check override first
        if let Some(name) = LEVEL_NAME_OVERRIDES
            .iter()
            .find(|&&(wi, ei, _)| wi == w && ei == e.entry_idx)
            .map(|&(_, _, name)| name)
        {
            e.name = name.to_string();
            continue;
        }

        e.name = match &e.kind {
            NodeKind::Start => format!("{w1}S"),
            NodeKind::Bowser => "8B".to_string(),
            NodeKind::Airship => format!("{w1}A"),
            NodeKind::Fortress { .. } => {
                fortress_ord[w] += 1;
                if fortress_counts[w] == 1 {
                    format!("{w1}F")
                } else {
                    format!("{w1}F{}", fortress_ord[w])
                }
            }
            NodeKind::Pipe { .. } => {
                pipe_ord[w] += 1;
                format!("{w1}Pi{}", pipe_ord[w])
            }
            NodeKind::ToadHouse => {
                toad_ord[w] += 1;
                if toad_counts[w] == 1 {
                    format!("{w1}TH")
                } else {
                    format!("{w1}TH{}", toad_ord[w])
                }
            }
            NodeKind::BonusGame => {
                bonus_ord[w] += 1;
                if bonus_counts[w] == 1 {
                    format!("{w1}BG")
                } else {
                    format!("{w1}BG{}", bonus_ord[w])
                }
            }
            NodeKind::HammerBro => {
                hammer_ord[w] += 1;
                if hammer_counts[w] == 1 {
                    format!("{w1}HB")
                } else {
                    format!("{w1}HB{}", hammer_ord[w])
                }
            }
            NodeKind::MapObject => {
                map_obj_ord[w] += 1;
                if map_obj_counts[w] == 1 {
                    format!("{w1}MO")
                } else {
                    format!("{w1}MO{}", map_obj_ord[w])
                }
            }
            NodeKind::Level => {
                // Numbered levels: tile 0x03-0x0F → level number = tile - 2
                if e.tile >= 0x03 && e.tile <= 0x0F {
                    format!("{w1}-{}", e.tile - 2)
                } else {
                    // Fallback for levels on non-numbered tiles
                    format!("{w1}L[{}]", e.entry_idx)
                }
            }
        };
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::randomize::rom_data::MAP_TILE_GRIDS;

    fn load_rom() -> Option<Rom> {
        let data = std::fs::read("Super Mario Bros. 3 (USA) (Rev 1).nes").ok()?;
        Rom::from_bytes(&data).ok()
    }

    #[test]
    fn test_total_count() {
        let rom = match load_rom() {
            Some(r) => r,
            None => return,
        };
        let catalog = NodeCatalog::build(&rom, false);
        let expected: usize = WORLDS.iter().map(|w| w.entry_count).sum();
        assert_eq!(catalog.entries.len(), expected, "expected {expected} total entries");
    }

    #[test]
    fn test_kind_counts() {
        let rom = match load_rom() {
            Some(r) => r,
            None => return,
        };
        let catalog = NodeCatalog::build(&rom, false);

        let count = |pred: fn(&NodeKind) -> bool| -> usize {
            catalog.entries.iter().filter(|e| pred(&e.kind)).count()
        };

        assert_eq!(count(|k| matches!(k, NodeKind::Fortress { .. })), 17, "fortresses");
        assert_eq!(count(|k| matches!(k, NodeKind::Airship)), 7, "airships");
        assert_eq!(count(|k| matches!(k, NodeKind::Bowser)), 1, "bowser");
        assert_eq!(count(|k| matches!(k, NodeKind::Start)), 8, "starts");
        assert_eq!(count(|k| matches!(k, NodeKind::Pipe { .. })), 48, "pipe endpoints");
    }

    #[test]
    fn test_pipe_pairs_consistent() {
        let rom = match load_rom() {
            Some(r) => r,
            None => return,
        };
        let catalog = NodeCatalog::build(&rom, false);

        // Every dest_idx should appear exactly twice
        let mut dest_counts: HashMap<usize, usize> = HashMap::new();
        for e in &catalog.entries {
            if let NodeKind::Pipe { dest_idx, .. } = &e.kind {
                *dest_counts.entry(*dest_idx).or_insert(0) += 1;
            }
        }

        for (&dest, &count) in &dest_counts {
            assert_eq!(count, 2, "dest_idx {dest} should appear exactly twice, got {count}");
        }
        assert_eq!(dest_counts.len(), 24, "should have 24 unique dest indices");
    }

    #[test]
    fn test_names_non_empty() {
        let rom = match load_rom() {
            Some(r) => r,
            None => return,
        };
        let catalog = NodeCatalog::build(&rom, false);

        for e in &catalog.entries {
            assert!(
                !e.name.is_empty(),
                "W{} entry {} has empty name",
                e.world_idx + 1, e.entry_idx,
            );
        }
    }

    #[test]
    fn test_grid_positions_valid() {
        let rom = match load_rom() {
            Some(r) => r,
            None => return,
        };
        let catalog = NodeCatalog::build(&rom, false);

        for e in &catalog.entries {
            let (row, col) = e.grid_pos;
            let max_cols = MAP_TILE_GRIDS[e.world_idx].columns;
            // Non-level entries may have row >= 9 (out of bounds) — that's fine,
            // they're classified as HammerBro. But level-like entries must be valid.
            if e.kind.is_level_like() {
                assert!(
                    row < 9 && col < max_cols,
                    "W{} {} ({:?}) at ({},{}) is out of bounds (max cols {})",
                    e.world_idx + 1, e.name, e.kind, row, col, max_cols,
                );
            }
        }
    }

    #[test]
    fn test_level_entry_presence() {
        let rom = match load_rom() {
            Some(r) => r,
            None => return,
        };
        let catalog = NodeCatalog::build(&rom, false);

        for e in &catalog.entries {
            if e.kind.is_level_like() {
                assert!(
                    e.level_entry.is_some(),
                    "W{} {} ({:?}) should have level_entry",
                    e.world_idx + 1, e.name, e.kind,
                );
            }
        }
    }

    #[test]
    fn test_fortress_boomboom_offsets() {
        let rom = match load_rom() {
            Some(r) => r,
            None => return,
        };
        let catalog = NodeCatalog::build(&rom, false);

        for e in &catalog.entries {
            if let NodeKind::Fortress { boomboom_y_offset } = &e.kind {
                assert!(
                    *boomboom_y_offset != 0,
                    "W{} {} has zero boomboom_y_offset",
                    e.world_idx + 1, e.name,
                );
            }
        }
    }

    #[test]
    fn test_regression_vs_old_classification() {
        let rom = match load_rom() {
            Some(r) => r,
            None => return,
        };
        let catalog = NodeCatalog::build(&rom, false);

        // Verify aggregate counts match known vanilla totals:
        // 17 fortresses, 7 airships, 1 bowser, 48 pipes, 8 starts
        let levels: usize = catalog.entries.iter()
            .filter(|e| matches!(e.kind, NodeKind::Level))
            .count();
        let fixed: usize = catalog.entries.iter()
            .filter(|e| matches!(
                e.kind,
                NodeKind::ToadHouse | NodeKind::BonusGame | NodeKind::HammerBro | NodeKind::MapObject
            ))
            .count();

        let total = levels + 17 + 48 + 7 + 1 + 8 + fixed;
        assert_eq!(total, 340, "total should be 340, got {total}");
    }

    /// Print the full catalog for visual inspection.
    /// Run with: cargo test -- test_print_catalog --ignored --nocapture
    #[test]
    #[ignore]
    fn test_print_catalog() {
        let rom = match load_rom() {
            Some(r) => r,
            None => {
                eprintln!("ROM not found, skipping");
                return;
            }
        };
        let catalog = NodeCatalog::build(&rom, false);

        let mut current_world = usize::MAX;
        for e in &catalog.entries {
            if e.world_idx != current_world {
                current_world = e.world_idx;
                eprintln!("\n=== World {} ({} entries) ===",
                    current_world + 1,
                    catalog.world(current_world).count(),
                );
            }

            let kind_str = match &e.kind {
                NodeKind::Level => "Level".to_string(),
                NodeKind::Fortress { boomboom_y_offset } =>
                    format!("Fortress(bb=0x{boomboom_y_offset:05X})"),
                NodeKind::Pipe { dest_idx, .. } => format!("Pipe(dest={dest_idx})"),
                NodeKind::Airship => "Airship".to_string(),
                NodeKind::Bowser => "Bowser".to_string(),
                NodeKind::Start => "Start".to_string(),
                NodeKind::ToadHouse => "ToadHouse".to_string(),
                NodeKind::BonusGame => "BonusGame".to_string(),
                NodeKind::HammerBro => "HammerBro".to_string(),
                NodeKind::MapObject => "MapObject".to_string(),
            };

            let entry_str = if let Some(le) = &e.level_entry {
                let obj = (le.obj_hi as u16) << 8 | le.obj_lo as u16;
                let lay = (le.lay_hi as u16) << 8 | le.lay_lo as u16;
                format!("obj=${obj:04X} lay=${lay:04X} ts={}", le.tileset)
            } else {
                "—".to_string()
            };

            eprintln!(
                "  [{:2}] {:8} ({:2},{:2})  tile=${:02X}  {}  {}",
                e.entry_idx, e.name, e.grid_pos.0, e.grid_pos.1,
                e.tile, kind_str, entry_str,
            );
        }

        // Summary
        eprintln!("\n=== Summary ===");
        let kind_names: &[(&str, fn(&NodeKind) -> bool)] = &[
            ("Level", |k| matches!(k, NodeKind::Level)),
            ("Fortress", |k| matches!(k, NodeKind::Fortress { .. })),
            ("Pipe", |k| matches!(k, NodeKind::Pipe { .. })),
            ("Airship", |k| matches!(k, NodeKind::Airship)),
            ("Bowser", |k| matches!(k, NodeKind::Bowser)),
            ("Start", |k| matches!(k, NodeKind::Start)),
            ("ToadHouse", |k| matches!(k, NodeKind::ToadHouse)),
            ("BonusGame", |k| matches!(k, NodeKind::BonusGame)),
            ("HammerBro", |k| matches!(k, NodeKind::HammerBro)),
            ("MapObject", |k| matches!(k, NodeKind::MapObject)),
        ];
        for (name, pred) in kind_names {
            let c: usize = catalog.entries.iter().filter(|e| pred(&e.kind)).count();
            eprintln!("  {name:12} {c}");
        }
        eprintln!("  Total:       {}", catalog.entries.len());
    }
}
