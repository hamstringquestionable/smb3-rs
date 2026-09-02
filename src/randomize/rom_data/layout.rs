//! The overworld map layout, **read from the ROM** rather than hardcoded.
//!
//! [`WORLDS`] and [`MAP_TILE_GRIDS`] are constants describing the vanilla
//! eight-world layout. That is fine for a vanilla ROM and wrong for any ROM
//! whose maps have been re-partitioned — the `mega_map` experiment folds the
//! eight worlds into three, and from that moment every consumer addressing the
//! map through those constants is reading the wrong offsets at the wrong
//! stride.
//!
//! Everything those constants say is already in the ROM: the master pointer
//! tables give each world's block, and `entry_count` falls out of the gap
//! between the RowType and ScrCol pointers, which are adjacent by
//! construction. So a single reader describes any ROM, and the vanilla
//! constants become one possible *output* rather than a parallel truth.
//!
//! The overworld pipeline is wired to this. [`NodeCatalog`] carries the layout
//! it was read against, and the pickup, build and write phases take it from
//! there rather than indexing the constants — which is why the catalog is the
//! right home for it: every later phase already receives one.
//!
//! Passes that run *before* any re-partitioning — the QoL map patches,
//! `credits`, testrom's `open_map` — legitimately keep using the constants,
//! because the vanilla layout is exactly what they are addressing.
//!
//! `layout_matches_the_vanilla_constants` pins that the reader agrees with the
//! constants on a vanilla ROM. That is what makes the migration safe, and it
//! was checked the strong way: a seeded randomizer run produces a
//! byte-identical ROM before and after.
//!
//! [`NodeCatalog`]: crate::randomize::node_catalog::NodeCatalog

use crate::rom::Rom;

use super::{PRG012_FILE_BASE, ROWS};

/// Grid pointer table: 9 little-endian CPU words (8 worlds + Warp Zone).
const GRID_PTRS: usize = 0x185A8;

/// Master pointer tables, one 16-bit CPU address per world.
const ROWTYPE_MASTER: usize = 0x193EC;
const SCRCOL_MASTER: usize = 0x193FE;

/// Bytes of tile data per page: 9 rows x 16 columns.
const PAGE_BYTES: usize = ROWS * 16;

/// `FortressFX_W1` — the per-world rows of FX slot indices.
const FX_WORLD_ROWS: usize = 0x14888;
/// `FortressFXBase_ByWorld` — byte offset into `FX_WORLD_ROWS` per world.
const FX_WORLD_BASE: usize = 0x148A8;

/// W5's spiral tower entries. It pairs like a pipe, so the pipe matcher needs
/// to know about it, but it wears castle tiles rather than pipe tiles.
const VANILLA_SPIRAL_ENTRIES: [(usize, usize); 2] = [(4, 10), (4, 21)];

/// One world's map layout as the ROM actually describes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WorldLayout {
    /// Engine world index, 0-based. Not necessarily its position in
    /// [`MapLayout::worlds`], once dead slots exist.
    pub slot: usize,
    /// File offset of the world's RowType sub-table.
    pub rowtype_offset: usize,
    /// Pointer entries in the world's block.
    pub entry_count: usize,
    /// File offset of the world's tile grid.
    pub grid_offset: usize,
    /// Pages (16-column screens) the grid holds.
    pub pages: usize,
}

impl WorldLayout {
    pub fn columns(&self) -> usize {
        self.pages * 16
    }

    /// File offset of the tile at `(row, col)`.
    ///
    /// Grids are stored **screen-major** — 144 bytes per page — so the obvious
    /// `base + row * columns + col` is wrong.
    pub fn tile_offset(&self, row: usize, col: usize) -> usize {
        self.grid_offset + (col / 16) * PAGE_BYTES + row * 16 + col % 16
    }
}

/// How to remap a `(world, entry)` pair when the map is re-partitioned.
pub(crate) type EntryRemap = std::collections::HashMap<(usize, usize), (usize, usize)>;

/// The live worlds of a ROM, and which of their entries are what.
///
/// The entry tables live here rather than staying constants because they are
/// keyed by `(world, entry_idx)` — the one thing a fold changes about every
/// carried entry. `BOOMBOOM_Y_OFFSETS` is deliberately absent: it is keyed by
/// `obj_ptr`, which the fold preserves, so it needs no remapping at all.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MapLayout {
    pub worlds: Vec<WorldLayout>,
    /// Entries that are fortresses.
    pub fortress_entries: Vec<(usize, usize)>,
    /// Entries that are airships — a world's goal.
    pub airship_entries: Vec<(usize, usize)>,
    /// The Bowser castle entry.
    pub bowser_entry: (usize, usize),
    /// W5's spiral tower pair, which pairs like a pipe but wears castle tiles.
    pub spiral_entries: Vec<(usize, usize)>,
    /// `(world, map-object slot, entry)` for sprites pinned to an entry.
    pub map_obj_entry_links: Vec<(usize, usize, usize)>,
    /// `(pipe destination index, world)`.
    pub dest_to_world: Vec<(u8, usize)>,
}

impl MapLayout {
    /// Read the layout of the given world slots.
    ///
    /// `live` names the slots that are real. It cannot be inferred: a folded
    /// ROM points its dead slots at a live world's tables rather than at
    /// garbage, which is safe precisely because they are never loaded — but it
    /// means the ROM alone cannot say which are which. The caller knows.
    pub fn read(rom: &Rom, live: &[usize]) -> Self {
        MapLayout {
            worlds: live.iter().map(|&slot| read_world(rom, slot)).collect(),
            fortress_entries: super::FORTRESS_ENTRIES.to_vec(),
            airship_entries: super::AIRSHIP_ENTRIES.to_vec(),
            bowser_entry: super::BOWSER_ENTRY,
            spiral_entries: VANILLA_SPIRAL_ENTRIES.to_vec(),
            map_obj_entry_links: super::MAP_OBJ_ENTRY_LINKS.to_vec(),
            dest_to_world: super::DEST_TO_WORLD.to_vec(),
        }
    }

    /// Move every entry table through a fold's `(world, entry)` remap.
    ///
    /// An entry the remap does not mention was not carried, and drops out —
    /// which is correct: a fortress on a page the fold left behind is not a
    /// fortress on this map. Positions are untouched; the fold preserves each
    /// entry's `obj_ptr`, so only its address changes.
    pub fn remapped(mut self, remap: &EntryRemap) -> Self {
        let one = |v: &Vec<(usize, usize)>| -> Vec<(usize, usize)> {
            v.iter().filter_map(|k| remap.get(k).copied()).collect()
        };
        self.fortress_entries = one(&self.fortress_entries);
        self.airship_entries = one(&self.airship_entries);
        self.spiral_entries = one(&self.spiral_entries);
        self.bowser_entry = remap.get(&self.bowser_entry).copied().unwrap_or(self.bowser_entry);
        self.map_obj_entry_links = self
            .map_obj_entry_links
            .iter()
            .filter_map(|&(w, slot, e)| remap.get(&(w, e)).map(|&(nw, ne)| (nw, slot, ne)))
            .collect();
        // A pipe destination belongs to whichever world its endpoints landed
        // in; both endpoints of a pair always move together.
        self.dest_to_world = self
            .dest_to_world
            .iter()
            .filter_map(|&(d, w)| {
                remap.iter().find(|((sw, _), _)| *sw == w).map(|(_, &(nw, _))| (d, nw))
            })
            .collect();
        self
    }

    /// Pipe destination indices belonging to a world.
    pub fn dest_indices_for_world(&self, slot: usize) -> Vec<usize> {
        self.dest_to_world.iter().filter(|&&(_, w)| w == slot).map(|&(d, _)| d as usize).collect()
    }

    /// FX slot indices per world, read from the ROM's own row table.
    ///
    /// Replaces the constant-driven reader, which assumed four bytes per world
    /// at a fixed stride. `FortressFXBase_ByWorld` gives each world's row
    /// offset directly, so packed rows of any length work — which is what a
    /// fold produces when one group owns seven fortresses.
    pub fn world_fx_assignments(&self, rom: &Rom) -> [Vec<u8>; 8] {
        let mut out: [Vec<u8>; 8] = Default::default();
        for slot in self.slots() {
            let forts = self.fortress_entries.iter().filter(|&&(w, _)| w == slot).count();
            let base = rom.read_byte(FX_WORLD_BASE + slot) as usize;
            out[slot] = (0..forts).map(|i| rom.read_byte(FX_WORLD_ROWS + base + i)).collect();
        }
        out
    }

    /// The vanilla eight-world layout, read from a vanilla ROM.
    pub fn vanilla(rom: &Rom) -> Self {
        Self::read(rom, &[0, 1, 2, 3, 4, 5, 6, 7])
    }

    pub fn get(&self, slot: usize) -> Option<&WorldLayout> {
        self.worlds.iter().find(|w| w.slot == slot)
    }

    /// The layout of a live world, panicking if the slot is dead.
    ///
    /// The mechanical replacement for `WORLDS[slot]` / `MAP_TILE_GRIDS[slot]`.
    /// Panicking is deliberate: indexing a dead slot is a bug in the caller's
    /// iteration, and the world-merge experiment showed that quietly reading a
    /// dead world's aliased tables produces plausible garbage rather than a
    /// crash — duplicated levels, entries at impossible rows — which is far
    /// harder to trace than a panic naming the slot.
    pub fn world(&self, slot: usize) -> &WorldLayout {
        self.get(slot).unwrap_or_else(|| panic!("map layout has no live world in slot {slot}"))
    }

    /// This world as a [`WorldTables`], so the existing `read_entry` /
    /// `table_offsets` helpers keep their signatures. The mechanical
    /// replacement for `&WORLDS[slot]`.
    pub fn tables(&self, slot: usize) -> super::WorldTables {
        let w = self.world(slot);
        super::WorldTables { rowtype_offset: w.rowtype_offset, entry_count: w.entry_count }
    }

    /// The live world slots, ascending. Replaces `0..8`.
    pub fn slots(&self) -> impl Iterator<Item = usize> + '_ {
        self.worlds.iter().map(|w| w.slot)
    }

    /// Read a live world's tile grid.
    ///
    /// The layout-aware [`read_tile_grid`], which sizes itself from the
    /// vanilla constants and so reads the wrong offsets at the wrong stride
    /// once the map has been re-partitioned.
    pub fn read_grid(&self, rom: &Rom, slot: usize) -> super::Grid {
        let w = self.world(slot);
        let tiles = (0..ROWS)
            .map(|r| (0..w.columns()).map(|c| rom.read_byte(w.tile_offset(r, c))).collect())
            .collect();
        super::Grid { tiles, cols: w.columns(), eights_are_wild: false }
    }

    /// How many worlds the game actually has. The point of the whole exercise:
    /// consumers ask this instead of assuming eight.
    // Reason(dead_code): the iteration sites migrated so far use `slots()`.
    // This is the count they need once the per-world budgets in
    // `overworld_build::capacity` stop being `[T; 8]`, which is the next step.
    #[allow(dead_code)]
    pub fn world_count(&self) -> usize {
        self.worlds.len()
    }
}

fn read_word(rom: &Rom, offset: usize) -> u16 {
    (rom.read_byte(offset) as u16) | ((rom.read_byte(offset + 1) as u16) << 8)
}

fn file_of(cpu: u16) -> usize {
    PRG012_FILE_BASE + (cpu as usize - 0xA000)
}

fn read_world(rom: &Rom, slot: usize) -> WorldLayout {
    let rowtype_offset = file_of(read_word(rom, ROWTYPE_MASTER + slot * 2));
    // ScrCol is laid out immediately after RowType, one byte per entry each,
    // so the gap between the two pointers *is* the entry count.
    let entry_count = file_of(read_word(rom, SCRCOL_MASTER + slot * 2)) - rowtype_offset;
    let grid_offset = file_of(read_word(rom, GRID_PTRS + slot * 2));

    // Pages come from the entries rather than from the grid data. Scanning
    // the grid for its `0xFF` terminator would be wrong: `0xFF` is also a
    // real map tile (the border), so a scan can stop early. Every page
    // carries at least one entry in both vanilla and the fold, so the highest
    // page any entry names is the last page.
    let scrcol = rowtype_offset + entry_count;
    let pages = (0..entry_count)
        .map(|i| (rom.read_byte(scrcol + i) >> 4) as usize)
        .max()
        .map_or(0, |p| p + 1);

    WorldLayout { slot, rowtype_offset, entry_count, grid_offset, pages }
}

#[cfg(test)]
mod tests {
    use super::super::{MAP_TILE_GRIDS, WORLDS};
    use super::*;

    fn vanilla() -> Option<Rom> {
        let data = std::fs::read("roms/Super Mario Bros. 3 (USA) (Rev 1).nes").ok()?;
        Rom::from_bytes(&data).ok()
    }

    /// The reader agrees with the hardcoded constants on a vanilla ROM.
    ///
    /// This is what lets call sites migrate off the constants one at a time:
    /// as long as it holds, the two describe the same thing.
    #[test]
    fn layout_matches_the_vanilla_constants() {
        let Some(rom) = vanilla() else { return };
        let layout = MapLayout::vanilla(&rom);

        assert_eq!(layout.world_count(), 8);
        for (slot, w) in layout.worlds.iter().enumerate() {
            assert_eq!(w.slot, slot);
            assert_eq!(
                w.rowtype_offset,
                WORLDS[slot].rowtype_offset,
                "W{} rowtype offset",
                slot + 1
            );
            assert_eq!(w.entry_count, WORLDS[slot].entry_count, "W{} entry count", slot + 1);
            assert_eq!(
                w.grid_offset,
                MAP_TILE_GRIDS[slot].file_offset,
                "W{} grid offset",
                slot + 1
            );
            assert_eq!(w.pages, MAP_TILE_GRIDS[slot].screens, "W{} page count", slot + 1);
            assert_eq!(w.columns(), MAP_TILE_GRIDS[slot].columns, "W{} columns", slot + 1);
        }
    }

    /// `tile_offset` is screen-major, matching `map_tile_offset`.
    #[test]
    fn tile_offset_is_screen_major() {
        let Some(rom) = vanilla() else { return };
        let layout = MapLayout::vanilla(&rom);
        for w in &layout.worlds {
            for row in 0..ROWS {
                for col in 0..w.columns() {
                    assert_eq!(
                        w.tile_offset(row, col),
                        super::super::map_tile_offset(w.slot, row, col),
                        "W{} ({row},{col})",
                        w.slot + 1
                    );
                }
            }
        }
    }

    /// The reader describes a folded ROM correctly, which the constants cannot.
    #[test]
    fn layout_reads_a_folded_rom() {
        use crate::randomize::mega_map;

        let Some(mut rom) = vanilla() else { return };
        mega_map::build(&mut rom).expect("fold");

        let live: Vec<usize> = mega_map::SUPER_WORLDS.iter().map(|s| s.slot).collect();
        let layout = MapLayout::read(&rom, &live);

        assert_eq!(layout.world_count(), 3, "three super-worlds");
        for sw in &mega_map::SUPER_WORLDS {
            let w = layout.get(sw.slot).expect("live slot present");
            assert_eq!(w.pages, mega_map::pages_in(sw.slot), "slot {} pages", sw.slot);
            assert!(w.entry_count > 0 && w.entry_count <= 255, "slot {} entries", sw.slot);
        }

        // Every entry names a page inside its world's grid.
        for w in &layout.worlds {
            let scrcol = w.rowtype_offset + w.entry_count;
            for i in 0..w.entry_count {
                let page = (rom.read_byte(scrcol + i) >> 4) as usize;
                assert!(page < w.pages, "slot {} entry {i} names page {page}", w.slot);
            }
        }
    }
}
