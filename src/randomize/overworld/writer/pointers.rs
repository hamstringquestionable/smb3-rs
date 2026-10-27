//! Step 3/5 — write pointer-table entries and pipe destination tables.

use super::*;

/// Write an entry's rowtype/screencol bytes: row nibble + tileset into the
/// rowtype table at `rt`, screen nibble + in-screen column into the
/// screen-column table at `sc`.
fn write_entry_pos(
    rom: &mut Rom,
    rt: usize,
    sc: usize,
    entry_idx: usize,
    pos: (usize, usize),
    tileset: u8,
) {
    let (screen, col_in_screen, row_nib) = pipe_helpers::grid_pos_to_dest_nibbles(pos.0, pos.1);
    rom.write_byte(rt + entry_idx, (row_nib << 4) | (tileset & 0x0F));
    rom.write_byte(sc + entry_idx, (screen << 4) | col_in_screen);
}

/// One world's pointer-table rows, once [`lay_out_rows`] has laid them out.
pub(super) struct Rows {
    /// The row of each entry pickup left in place, by its vanilla `entry_idx`.
    kept: HashMap<usize, usize>,
    /// The rows for [`write_pointer_entries`] to fill, in fill order.
    free: Vec<usize>,
}

/// Lay the pointer tables out with each world's kept entries first and then
/// the rows its map asks for (`ClearedWorld::free_rows`), and say where they
/// sit. Where a row starts does not matter: `resort_pointer_table` puts every
/// world back in screen/row order afterwards.
pub(super) fn lay_out_rows(rom: &mut Rom, data: &OverworldData) -> Vec<Rows> {
    let picked: HashSet<usize> = data.pickup.pool.iter().map(|pe| pe.catalog_idx).collect();
    let kept: Vec<Vec<usize>> = (0..8)
        .map(|wi| {
            let entries = data.catalog.entries.iter().enumerate();
            entries
                .filter(|(ci, e)| e.world_idx == wi && !picked.contains(ci))
                .map(|(_, e)| e.entry_idx)
                .collect()
        })
        .collect();
    let counts: [usize; 8] =
        std::array::from_fn(|wi| kept[wi].len() + data.pickup.worlds[wi].free_rows);

    pipe_helpers::relayout_pointer_tables(rom, &kept, &counts);
    (0..8)
        .map(|wi| Rows {
            kept: kept[wi].iter().enumerate().map(|(row, &i)| (i, row)).collect(),
            free: (kept[wi].len()..counts[wi]).collect(),
        })
        .collect()
}

/// Write the position of every entry pickup left in place whose position on
/// the map differs from the ROM's: the start and airship of a world Start ↔
/// Airship swap flipped, and anything a generated map moved. Entries pickup
/// took are written by [`write_pointer_entries`] instead, so they are skipped.
///
/// Must run BEFORE `pipe_helpers::resort_pointer_table` for the same world, so
/// the resort puts the rewritten entries in their screen/row order.
pub(super) fn write_kept_entry_positions(
    rom: &mut Rom,
    world_idx: usize,
    data: &OverworldData,
    rows: &Rows,
) {
    let world = &rom_data::world_tables(rom, world_idx);
    let rt = world.rowtype_offset;
    let sc = rt + world.entry_count;
    for entry in &data.catalog.entries {
        let Some(&row) = rows.kept.get(&entry.entry_idx).filter(|_| entry.world_idx == world_idx)
        else {
            continue;
        };
        if rom_data::entry_grid_position(rom, world, row) == entry.grid_pos {
            continue;
        }
        // Keep the low nibble of the rowtype byte: it is the tileset.
        let tileset = rom.read_byte(rt + row);
        write_entry_pos(rom, rt, sc, row, entry.grid_pos, tileset);
    }
}

pub(super) fn write_pointer_entries(
    rom: &mut Rom,
    world_idx: usize,
    built: &BuiltWorld,
    wa: &WorldAssignments,
    data: &OverworldData,
    hb_level_iter: &mut impl Iterator<Item = rom_data::LevelEntry>,
    rows: &Rows,
) {
    let pickup = data.pickup;
    let catalog = data.catalog;
    let world = &rom_data::world_tables(rom, world_idx);
    let n = world.entry_count;
    let rt = world.rowtype_offset;
    let sc = rt + n;

    // The rows to fill: the ones pickup vacated, or the free rows of a new layout.
    let available_slots = &rows.free;

    // Collect all assignments as (pool_idx, pos) for level-like entries.
    let mut all: Vec<(usize, (usize, usize))> = Vec::new();

    for a in &wa.fortress {
        all.push((a.pool_idx, a.pos));
    }
    for a in &wa.level {
        all.push((a.pool_idx, a.pos));
    }
    for pa in &wa.pipes {
        all.push((pa.pool_idx_a, pa.pos_a));
        all.push((pa.pool_idx_b, pa.pos_b));
    }
    for a in &wa.bonus {
        all.push((a.pool_idx, a.pos));
    }
    for a in &wa.toad {
        all.push((a.pool_idx, a.pos));
    }
    // Airship and Bowser are never picked up: they are kept entries, which
    // `write_kept_entry_positions` moves.

    // A hard assert, not a debug one: an entry dropped here leaves a stamped
    // tile with no pointer behind it, which crashes the game (see below).
    assert!(
        all.len() + wa.hammer_bro.len() <= available_slots.len(),
        "W{}: slot overflow: need {} but only {} available",
        world_idx + 1,
        all.len() + wa.hammer_bro.len(),
        available_slots.len(),
    );

    let mut slot_i = 0;

    // Write level-like entries (fortress, level, pipe).
    for &(pool_idx, pos) in &all {
        let entry_idx = available_slots[slot_i];
        slot_i += 1;

        let pe = &pickup.pool[pool_idx];
        let ce = &catalog.entries[pe.catalog_idx];
        let level_entry =
            ce.level_entry.as_ref().expect("assigned pool entry must have level_entry");

        rom_data::write_entry(rom, world, entry_idx, level_entry);
        write_entry_pos(rom, rt, sc, entry_idx, pos, level_entry.tileset);
    }

    // Write hammer bro entries (carry their own LevelEntry).
    for hb in &wa.hammer_bro {
        let entry_idx = available_slots[slot_i];
        slot_i += 1;

        rom_data::write_entry(rom, world, entry_idx, &hb.level_entry);
        write_entry_pos(rom, rt, sc, entry_idx, hb.pos, hb.level_entry.tileset);
    }

    // Fill any remaining unused pointer table slots with valid HB levels.
    // These are blank node tiles on the grid that weren't assigned slots
    // during the build phase (e.g., not BFS-reachable at build time).
    // Place them at actual blank positions so the player doesn't walk onto
    // a tile with no pointer entry (which crashes the game).
    if slot_i < available_slots.len() {
        // Collect positions already covered by assignments above.
        let mut covered: HashSet<(usize, usize)> = HashSet::new();
        for &(_, pos) in &all {
            covered.insert(pos);
        }
        for hb in &wa.hammer_bro {
            covered.insert(hb.pos);
        }

        // Find blank tile positions on the grid that have no entry.
        // Exclude positions of catalog entries that were never picked up
        // (airship, Bowser, map objects like piranhas, start). These already
        // have valid pointer table entries from vanilla, so filling them
        // wastes a slot that should go to a real uncovered blank.
        let already_has_entry: HashSet<(usize, usize)> = catalog
            .entries
            .iter()
            .filter(|e| {
                e.world_idx == world_idx
                    && !matches!(
                        e.kind,
                        NodeKind::Level
                            | NodeKind::Fortress
                            | NodeKind::Pipe { .. }
                            | NodeKind::HammerBro
                            | NodeKind::BonusGame
                            | NodeKind::ToadHouse
                    )
            })
            .map(|e| e.grid_pos)
            .collect();
        let mut uncovered_blanks: Vec<(usize, usize)> = Vec::new();
        for r in 0..built.grid.rows() {
            for c in 0..built.grid.cols {
                if rom_data::VALID_BLANK_TILES.contains(&built.grid.get(r, c))
                    && !covered.contains(&(r, c))
                    && !already_has_entry.contains(&(r, c))
                {
                    uncovered_blanks.push((r, c));
                }
            }
        }
        let mut blank_iter = uncovered_blanks.into_iter();

        while slot_i < available_slots.len() {
            let entry_idx = available_slots[slot_i];
            slot_i += 1;
            let le = hb_level_iter.next().unwrap();
            rom_data::write_entry(rom, world, entry_idx, &le);

            if let Some(pos) = blank_iter.next() {
                // Place at actual blank tile position.
                write_entry_pos(rom, rt, sc, entry_idx, pos, le.tileset);
            } else {
                // No more blanks — park at unreachable position.
                rom.write_byte(rt + entry_idx, le.tileset & 0x0F); // row_nib=0 → grid_row=-2
                rom.write_byte(sc + entry_idx, 0x00);
            }
        }
    }
}

pub(super) fn write_pipe_dests(rom: &mut Rom, world_idx: usize, wa: &WorldAssignments) {
    for pa in &wa.pipes {
        pipe_helpers::write_pipe_dest(rom, pa.dest_idx, pa.pos_a, pa.pos_b, world_idx);
    }
}
