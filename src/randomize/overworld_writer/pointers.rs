//! Step 3/5 — write pointer-table entries and pipe destination tables.

use super::*;

pub(super) fn write_pointer_entries(
    rom: &mut Rom,
    world_idx: usize,
    built: &BuiltWorld,
    wa: &WorldAssignments,
    data: &OverworldData,
    hb_level_iter: &mut impl Iterator<Item = rom_data::LevelEntry>,
) {
    let pickup = data.pickup;
    let catalog = data.catalog;
    let world = &WORLDS[world_idx];
    let n = world.entry_count;
    let rt = world.rowtype_offset;
    let sc = rt + n;

    // Reusable entry_idx values: pointer table slots vacated during Phase 2 pickup.
    let cw = &pickup.worlds[world_idx];
    let available_slots: Vec<usize> = cw
        .pool_indices
        .iter()
        .map(|&pi| pickup.pool[pi].entry_idx)
        .collect();

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
    // Airship and bowser are not picked up — their pointer table entries
    // stay vanilla so the autoscroll patch's hardcoded offsets remain valid.

    debug_assert!(
        all.len() + wa.hammer_bro.len() <= available_slots.len(),
        "W{}: slot overflow: need {} but only {} available",
        world_idx + 1,
        all.len() + wa.hammer_bro.len(),
        available_slots.len(),
    );

    let mut slot_i = 0;

    // Write level-like entries (fortress, level, pipe).
    for &(pool_idx, pos) in &all {
        if slot_i >= available_slots.len() {
            break;
        }
        let entry_idx = available_slots[slot_i];
        slot_i += 1;

        let pe = &pickup.pool[pool_idx];
        let ce = &catalog.entries[pe.catalog_idx];
        let level_entry = ce
            .level_entry
            .as_ref()
            .expect("assigned pool entry must have level_entry");

        rom_data::write_entry(rom, world, entry_idx, level_entry);

        let (row, col) = pos;
        let row_nib = (row + 2) as u8;
        let screen = (col / 16) as u8;
        let col_in_screen = (col % 16) as u8;

        rom.write_byte(rt + entry_idx, (row_nib << 4) | (level_entry.tileset & 0x0F));
        rom.write_byte(sc + entry_idx, (screen << 4) | col_in_screen);
    }

    // Write hammer bro entries (carry their own LevelEntry).
    for hb in &wa.hammer_bro {
        if slot_i >= available_slots.len() {
            break;
        }
        let entry_idx = available_slots[slot_i];
        slot_i += 1;

        rom_data::write_entry(rom, world, entry_idx, &hb.level_entry);

        let (row, col) = hb.pos;
        let row_nib = (row + 2) as u8;
        let screen = (col / 16) as u8;
        let col_in_screen = (col % 16) as u8;

        rom.write_byte(rt + entry_idx, (row_nib << 4) | (hb.level_entry.tileset & 0x0F));
        rom.write_byte(sc + entry_idx, (screen << 4) | col_in_screen);
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
        let already_has_entry: HashSet<(usize, usize)> = catalog.entries.iter()
            .filter(|e| e.world_idx == world_idx && !matches!(e.kind,
                NodeKind::Level | NodeKind::Fortress { .. }
                | NodeKind::Pipe { .. } | NodeKind::HammerBro
                | NodeKind::BonusGame | NodeKind::ToadHouse))
            .map(|e| e.grid_pos)
            .collect();
        let mut uncovered_blanks: Vec<(usize, usize)> = Vec::new();
        for r in 0..built.grid.rows {
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

            if let Some((row, col)) = blank_iter.next() {
                // Place at actual blank tile position.
                let row_nib = (row + 2) as u8;
                let screen = (col / 16) as u8;
                let col_in_screen = (col % 16) as u8;
                rom.write_byte(rt + entry_idx, (row_nib << 4) | (le.tileset & 0x0F));
                rom.write_byte(sc + entry_idx, (screen << 4) | col_in_screen);
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
