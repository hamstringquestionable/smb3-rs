//! Overworld map tile edits: rocks, W8 canoe/bridges, drawbridges, N-cards.

use crate::rom::Rom;
use crate::randomize::rom_data::{FX_MAP_TILE_REPLACE, map_tile_offset, write_map_sprite};

// W3 drawbridge map tile patches: (file offset, replacement tile).
// Vanilla: 2× $B2 horizontal + 2× $B1 vertical. Replace with $B3
// (horizontal bridge path) and $BA (vertical-compatible open path).
const W3_DRAWBRIDGE_TILES: [(usize, u8); 4] = [
    (0x18777, 0xB3), // H1
    (0x18779, 0xB3), // H2
    (0x1880C, 0xBA), // V1
    (0x188F3, 0xBA), // V2
];

// Toggle code: LDA $07BB; EOR #$01; STA $07BB (8 bytes at 0x14A68)
const W3_TOGGLE_OFFSET: usize = 0x14A68;
const W3_TOGGLE_LEN: usize = 8;

// W2 rock blocking secret path (screen 1, row 0, col 5) — $51 → $45
const W2_SECRET_ROCK: usize = 0x186E0;

// W3 rock blocking boat path (screen 0, row 6, col 15) — $51 → $45
const W3_BOAT_ROCK: usize = 0x187DB;

// W4 rock blocking pipe path (screen 1, row 6, col 25) — $51 → $45
const W4_PIPE_ROCK: usize = 0x18A16;

// W1 (6,5) decoration tile between nodes 14 and 20 — vanilla 0x53 is
// visually a rock but blocks all directions and is not registered as
// removable. Writing 0x51 here turns it into a real hammer rock: it
// becomes path 0x45 when broken (by `hammer_breaks_tiles`), cleared by
// `remove_rocks`, and auto-cleared by vanilla after the W1 fortress.
const W1_HAMMER_ROCK_OFFSET: usize = 0x1861F;

/// Remove the W2 secret-path, W3 boat-path, and W4 pipe-shortcut rocks,
/// replacing each with a horizontal path tile.
pub fn remove_rocks(rom: &mut Rom) {
    for offset in [W2_SECRET_ROCK, W3_BOAT_ROCK, W4_PIPE_ROCK] {
        rom.write_byte(offset, 0x45);
    }
}

/// W8 (Dark World) canoe + extra-path edits, gated behind the `8s are Wild`
/// option (see [`apply_w8_canoe_and_paths`]).
///
/// Each entry is `(row, global_col, tile)`, stamped into the W8 tile grid
/// before the overworld builder picks it up so the builder's BFS sees the new
/// connectivity. `0x4B` is the canoe dock tile.
///
/// (5,6) is the mainland dock; the canoe sprite floats at (5,7) beside it, and
/// (3,8)/(5,10)/(5,12) are island docks reachable by canoe (see
/// `active_canoe_edges`). The vanilla lock at (2,8) is intentionally NOT removed
/// here — the builder opens that FX slot during pickup.
///
/// The screen-2 hammer-breakable rock at (3,37) is NOT here: it belongs to the
/// `More hammer rocks` option (see [`make_hammer_rocks`]) and is placed
/// independently of this flag.
const W8_CANOE_PATH_EDITS: &[(usize, usize, u8)] = &[
    // --- Screen 0: canoe docks + navy approach ---
    (3, 8, 0x4B), (3, 10, 0x44), (3, 12, 0x44),
    (4, 8, 0x85), (4, 10, 0x46), (4, 12, 0x46),
    (5, 6, 0x4B), (5, 7, 0x8C), (5, 8, 0x8D), (5, 10, 0x4B), (5, 12, 0x4B),
    // --- Screen 2: extra paths ---
    (1, 36, 0x44), (1, 37, 0x45), (1, 38, 0x45), (1, 39, 0x45), (1, 40, 0x47),
    (1, 41, 0x45), (1, 42, 0x47), (1, 43, 0x45), (1, 44, 0x47), (1, 45, 0x45),
    (1, 46, 0x47),
    (2, 36, 0x46), (2, 46, 0x46),
    (3, 36, 0x4A), (3, 46, 0x48),
    (4, 46, 0x46),
    (5, 45, 0x45), (5, 46, 0x4A),
];

/// W8 (Dark World) screen-3 water + bridge edits, always applied. The bridge
/// tiles (`0xB3`) on the final page get gated as water gaps (`gap_tile_for`:
/// `0xB3 -> 0x9D`) by the builder instead of locks. See [`apply_w8_bridges`].
const W8_BRIDGE_EDITS: &[(usize, usize, u8)] = &[
    // --- Screen 3: water (row 4) + bridges (row 5) on the final page ---
    (4, 51, 0x99), (4, 52, 0xA2), (4, 53, 0x83), (4, 54, 0xA2), (4, 55, 0x83),
    (4, 56, 0xA2), (4, 57, 0x83), (4, 58, 0xA2), (4, 59, 0x9A),
    (5, 51, 0xB3), (5, 53, 0xB3), (5, 55, 0xB3), (5, 57, 0xB3), (5, 59, 0xB3),
];

/// Apply the always-on W8 screen-3 water + bridge approach (see
/// [`W8_BRIDGE_EDITS`]). Independent of the `8s are Wild` option.
pub fn apply_w8_bridges(rom: &mut Rom) {
    for &(row, col, tile) in W8_BRIDGE_EDITS {
        rom.write_byte(map_tile_offset(7, row, col), tile);
    }
    // Vanilla FX slot 16 sits at W8 (row 5, col 53) — right on our new bridge
    // row — and its replace_tile is 0x45 (plain path). The builder's pickup
    // `open_fx_gaps()` stamps that replace_tile over the grid, clobbering our
    // 0xB3 bridge. Point it at the bridge tile so the slot opens to a bridge,
    // matching the other bridge columns (and gating as a water gap if a
    // fortress lands there).
    rom.write_byte(FX_MAP_TILE_REPLACE + 16, 0xB3);
}

/// Apply the W8 canoe docks + extra paths and place the canoe sprite (see
/// [`W8_CANOE_PATH_EDITS`]). Gated behind the `8s are Wild` option.
pub fn apply_w8_canoe_and_paths(rom: &mut Rom) {
    for &(row, col, tile) in W8_CANOE_PATH_EDITS {
        rom.write_byte(map_tile_offset(7, row, col), tile);
    }
    // Place the W8 canoe (object ID 0x10) in map-object slot 6, floating at
    // (5,7) beside the mainland dock (5,6). Slot 6 is past the builder's army
    // sprites (slots 2-5), so the overworld writer leaves it intact. This is
    // the boat that makes the screen-0 island docks reachable in-game.
    write_map_sprite(rom, 7, 6, 5, 7, 0x10);
}

/// Add extra hammer-breakable rocks (the `More hammer rocks` option).
///
/// - **W1 (6,5):** vanilla puts 0x53 (visually a rock, blocks all directions,
///   not removable) at the gap between hammer-bro node 14 and toad house node
///   20. Writing 0x51 keeps the same visual but registers the tile as a real
///   "removable" rock, so it integrates with `hammer_breaks_tiles`,
///   `remove_rocks`, and vanilla fortress-clear behavior just like the
///   W2/W3/W4/W6 rocks.
/// - **W8 (3,37):** a screen-2 hammer-breakable rock. It sits on the vanilla
///   map (its west neighbor (3,36) is already a path) and is placed
///   independently of the `8s are Wild` option.
pub fn make_hammer_rocks(rom: &mut Rom) {
    rom.write_byte(W1_HAMMER_ROCK_OFFSET, 0x51);
    rom.write_byte(map_tile_offset(7, 3, 37), 0x51);
}

/// Remove N-card (N-Spade) panels from the overworld map.
///
/// Patches the map-screen code so N-Spade tiles never appear.
/// Original IPS: 3 bytes at 0x016C90 → LDA #$07; NOP.
const N_CARD_OFFSET: usize = 0x016C90;

pub fn remove_n_cards(rom: &mut Rom) {
    rom.write_range(N_CARD_OFFSET, &[0xA9, 0x07, 0xEA]);
}

/// Replace W3 drawbridge tiles with normal path tiles and NOP the toggle code.
pub fn fix_w3_drawbridges(rom: &mut Rom) {
    for (offset, tile) in W3_DRAWBRIDGE_TILES {
        rom.write_byte(offset, tile);
    }
    // NOP out the toggle code (LDA $07BB; EOR #$01; STA $07BB)
    rom.write_range(W3_TOGGLE_OFFSET, &[0xEA; W3_TOGGLE_LEN]);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::randomize::qol::test_support::make_test_rom;

    #[test]
    fn test_remove_rocks() {
        let mut rom = make_test_rom();
        for offset in [W2_SECRET_ROCK, W3_BOAT_ROCK, W4_PIPE_ROCK] {
            rom.write_byte(offset, 0x51);
        }
        remove_rocks(&mut rom);
        for offset in [W2_SECRET_ROCK, W3_BOAT_ROCK, W4_PIPE_ROCK] {
            assert_eq!(rom.read_byte(offset), 0x45);
        }
    }

    #[test]
    fn test_make_hammer_rocks() {
        let mut rom = make_test_rom();
        rom.write_byte(W1_HAMMER_ROCK_OFFSET, 0x53);
        make_hammer_rocks(&mut rom);
        // W1 (6,5) rock.
        assert_eq!(rom.read_byte(W1_HAMMER_ROCK_OFFSET), 0x51);
        // W8 (3,37) screen-2 rock.
        assert_eq!(
            rom.read_byte(map_tile_offset(7, 3, 37)),
            0x51
        );
    }

    #[test]
    fn test_remove_n_cards() {
        let mut rom = make_test_rom();
        rom.write_range(N_CARD_OFFSET, &[0x00, 0x00, 0x00]);
        remove_n_cards(&mut rom);
        assert_eq!(rom.read_range(N_CARD_OFFSET, 3), &[0xA9, 0x07, 0xEA]);
    }

    #[test]
    fn test_fix_w3_drawbridges() {
        let mut rom = make_test_rom();
        for (offset, _) in W3_DRAWBRIDGE_TILES {
            rom.write_byte(offset, 0x00);
        }
        rom.write_range(W3_TOGGLE_OFFSET, &[0xAD, 0xBB, 0x07, 0x49, 0x01, 0x8D, 0xBB, 0x07]);

        fix_w3_drawbridges(&mut rom);

        for (offset, tile) in W3_DRAWBRIDGE_TILES {
            assert_eq!(rom.read_byte(offset), tile);
        }
        assert_eq!(rom.read_range(W3_TOGGLE_OFFSET, W3_TOGGLE_LEN), &[0xEA; 8]);
    }
}
