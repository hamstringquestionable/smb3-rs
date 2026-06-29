//! Step 2 — stamp the per-world map tile grids.

use super::*;

pub(super) fn write_tile_grid<R: Rng>(
    rom: &mut Rom,
    built: &BuiltWorld,
    wa: &WorldAssignments,
    data: &OverworldData,
    sprite_mask: &HashSet<(usize, usize)>,
    rng: &mut R,
) {
    let pickup = data.pickup;
    let catalog = data.catalog;
    let wi = built.world_idx;
    let mut grid = built.grid.clone();

    // Stamp fortress tiles. The game treats $67, $EB, and $6A as fortress
    // tiles (Map_Removable_Tiles + completion-unsafe), so pick per-fortress.
    // $6A's CHR animation is frozen by patch_metatile_6a_freeze.
    const FORTRESS_TILES: [u8; 3] = [0x67, 0xEB, 0x6A];
    for a in &wa.fortress {
        let tile = FORTRESS_TILES[rng.random_range(..FORTRESS_TILES.len())];
        grid.set(a.pos.0, a.pos.1, tile);
    }

    // Stamp pipe tiles (handle spiral castle $5F).
    for pa in &wa.pipes {
        let tile_a = catalog.entries[pickup.pool[pa.pool_idx_a].catalog_idx].tile;
        let tile_b = catalog.entries[pickup.pool[pa.pool_idx_b].catalog_idx].tile;
        grid.set(pa.pos_a.0, pa.pos_a.1, if tile_a == 0x5F { 0x5F } else { TILE_PIPE });
        grid.set(pa.pos_b.0, pa.pos_b.1, if tile_b == 0x5F { 0x5F } else { TILE_PIPE });
    }

    // Stamp airship tile.
    if let Some(a) = &wa.airship {
        let tile = catalog.entries[pickup.pool[a.pool_idx].catalog_idx].tile;
        grid.set(a.pos.0, a.pos.1, tile);
    }

    // Stamp bowser tile.
    if let Some(a) = &wa.bowser {
        let tile = catalog.entries[pickup.pool[a.pool_idx].catalog_idx].tile;
        grid.set(a.pos.0, a.pos.1, tile);
    }

    // Stamp bonus game (spade) tiles.
    for a in &wa.bonus {
        grid.set(a.pos.0, a.pos.1, TILE_BONUS_GAME);
    }

    // Stamp toad house tiles. 0x50 and 0xE0 each carry their own embedded
    // background, and only in W5 do the two pages have different background
    // graphics — page 0 matches 0x50, page 1 matches 0xE0 — so the byte
    // choice has to follow position there. In every other world both
    // variants render against the same world background, so the visual is
    // identical either way and we just preserve the entry's vanilla tile.
    for a in &wa.toad {
        let tile = if wi == 4 {
            if a.pos.1 >= 16 { 0xE0 } else { 0x50 }
        } else {
            catalog.entries[pickup.pool[a.pool_idx].catalog_idx].tile
        };
        grid.set(a.pos.0, a.pos.1, tile);
    }

    // Stamp level tiles in BFS order from start.
    let level_pos_set: HashMap<(usize, usize), usize> = wa
        .level
        .iter()
        .enumerate()
        .map(|(i, a)| (a.pos, i))
        .collect();

    let start_pos = rom_data::find_start(&grid);
    let bfs = bfs_ordered(&grid, &built.pipe_pairs, start_pos, built.world_idx);

    // Level tile sequence: $03-$0B = levels 1-9 (vanilla numbered tiles),
    // $0C-$15 = levels 10-19 (double-digit tiles with custom "1" tens digit
    // patched by patch_double_digit_metatiles). $69 (pyramid) is a level-20+
    // fallback with no valid display.
    const LEVEL_TILES: [u8; 20] = [
        0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0A, 0x0B,
        0x0C, 0x0D, 0x0E, 0x0F, 0x10, 0x11, 0x12, 0x13, 0x14,
        0x15, 0x69,
    ];

    // 0xE6 (HANDTRAP) — visible hand-trap tile. Stamped instead of a level
    // number when the build has flagged this slot as a hand trap. Vanilla's
    // post-arrival CMP at $CF15 fires the grab dispatch (forced 100% by
    // hands_levels::install_full_grab); the slot's level pointer entry is
    // unchanged so the player drops into the underlying level.
    const TILE_HAND_TRAP: u8 = 0xE6;
    // 0xBC (PIPE) — visible pipe tile. Stamped instead of a level number
    // when the build has flagged this slot as a troll pipe. Pressing A on
    // the pipe matches Map_EnterSpecialTiles and falls through to the same
    // Map_Operation = $10 "enter level" path used by level number tiles, so
    // the slot's level pointer entry loads as a regular level.
    const TILE_TROLL_PIPE: u8 = 0xBC;
    let hand_trap_positions: HashSet<(usize, usize)> = built
        .slots
        .iter()
        .filter(|s| s.is_hand_trap)
        .map(|s| s.pos)
        .collect();
    let troll_pipe_positions: HashSet<(usize, usize)> = built
        .slots
        .iter()
        .filter(|s| s.is_troll_pipe)
        .map(|s| s.pos)
        .filter(|pos| !wa.demoted_troll_pipes.contains(pos))
        .collect();

    let mut level_idx: usize = 0;
    let mut assigned: Vec<bool> = vec![false; wa.level.len()];

    let pick_level_tile = |pos: (usize, usize), level_idx: &mut usize| -> u8 {
        if hand_trap_positions.contains(&pos) {
            TILE_HAND_TRAP
        } else if troll_pipe_positions.contains(&pos) {
            TILE_TROLL_PIPE
        } else {
            let t = LEVEL_TILES[(*level_idx).min(LEVEL_TILES.len() - 1)];
            *level_idx += 1;
            t
        }
    };

    for &(pos, _dist) in &bfs {
        if let Some(&la_idx) = level_pos_set.get(&pos) && !assigned[la_idx] {
            let tile = pick_level_tile(pos, &mut level_idx);
            grid.set(pos.0, pos.1, tile);
            assigned[la_idx] = true;
        }
    }

    // Any level slots not reached by BFS (safety fallback).
    for (i, a) in wa.level.iter().enumerate() {
        if !assigned[i] {
            let tile = pick_level_tile(a.pos, &mut level_idx);
            grid.set(a.pos.0, a.pos.1, tile);
        }
    }

    // Stamp lock gap tiles.
    for lock in &built.locks {
        grid.set(lock.pos.0, lock.pos.1, lock.gap_tile);
    }

    // Overwrite sprite-covered positions with connectivity-aware path nodes.
    // W8 army sprites float on top of the grid; the underlying tile must be
    // a plain path node, not a fortress/level tile. Skip BLANK_TILE_OVERRIDES
    // since sprite positions are dynamic (not the vanilla fixed positions).
    for &pos in sprite_mask {
        let tile = crate::randomize::overworld_pickup::blank_tile_from_neighbors(&grid, wi, pos.0, pos.1);
        grid.set(pos.0, pos.1, tile);
    }

    // Write grid to ROM.
    for r in 0..grid.rows {
        for c in 0..grid.cols {
            let offset = rom_data::map_tile_offset(wi, r, c);
            rom.write_byte(offset, grid.get(r, c));
        }
    }
}
