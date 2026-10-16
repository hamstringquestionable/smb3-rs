//! Edits the randomizer makes to vanilla's maps before anything reads them:
//! drawbridges made permanent, rocks removed or added, the W1 shortcut, and
//! the W8 bridges, water and canoe docks.
//!
//! They are applied to the in-memory [`WorldMap`]s, never to the ROM; the
//! overworld writer puts every world's grid in place once, at the end. The one
//! exception is the W8 canoe *sprite*, which `qol::place_w8_canoe` still writes
//! to the ROM, because map sprites have several writers.

use crate::randomize::rom_data::{BRIDGE_TILE, Pos, W8_IDX};

use super::WorldMap;

/// Apply every vanilla map edit, in the order the pipeline has always used.
pub(super) fn apply(maps: &mut [WorldMap], more_hammer_rocks: bool, eights_are_wild: bool) {
    stamp(maps, W3_DRAWBRIDGE_TILES);
    stamp(maps, REMOVED_ROCKS);
    if more_hammer_rocks {
        stamp(maps, HAMMER_ROCKS);
    }
    apply_w1_shortcut(maps, more_hammer_rocks);
    apply_w8_bridges(maps);
    if eights_are_wild {
        for &(row, col, tile) in W8_CANOE_PATH_EDITS {
            maps[W8_IDX].grid.set(row, col, tile);
        }
    }
}

/// Set each `(world, (row, col), tile)` on its world's grid.
fn stamp(maps: &mut [WorldMap], edits: &[(usize, Pos, u8)]) {
    for &(world, (row, col), tile) in edits {
        maps[world].grid.set(row, col, tile);
    }
}

/// W3's drawbridges, made permanent. Vanilla: 2× `$B2` horizontal + 2× `$B1`
/// vertical. Replaced with `$B3` (horizontal bridge path) and `$BA`
/// (vertical-compatible open path). The engine half, freezing the
/// `World3_Bridge` toggle, is `qol::fix_w3_drawbridges`.
pub(crate) const W3_DRAWBRIDGE_TILES: &[(usize, Pos, u8)] = &[
    (2, (0, 11), 0xB3), // H1
    (2, (0, 13), 0xB3), // H2
    (2, (1, 16), 0xBA), // V1
    (2, (6, 39), 0xBA), // V2
];

/// The W2 secret-path, W3 boat-path, and W4 pipe-shortcut rocks, each replaced
/// with a horizontal path tile. Always removed: the overworld builder relies
/// on those tiles being open for connectivity.
pub(crate) const REMOVED_ROCKS: &[(usize, Pos, u8)] = &[
    (1, (0, 21), 0x45), // W2 rock blocking the secret path
    (2, (6, 15), 0x45), // W3 rock blocking the boat path
    (3, (6, 25), 0x45), // W4 rock blocking the pipe path
];

/// Extra hammer-breakable rocks (the `More hammer rocks` option).
///
/// - **W1 (6,5):** vanilla puts 0x53 (visually a rock, blocks all directions,
///   not removable) at the gap between hammer-bro node 14 and toad house node
///   20. Writing 0x51 keeps the same visual but registers the tile as a real
///   "removable" rock, so it integrates with `hammer_breaks_tiles`, the
///   removed rocks above, and vanilla fortress-clear behavior just like the
///   W2/W3/W4/W6 rocks. It becomes path 0x45 when broken, and vanilla clears
///   it after the W1 fortress.
/// - **W8 (3,37):** a screen-2 hammer-breakable rock. It sits on the vanilla
///   map (its west neighbor (3,36) is already a path) and is placed
///   independently of the `8s are Wild` option.
///
/// The W1 (5,8) shortcut rock is also gated by this option, but it lives in
/// [`apply_w1_shortcut`] — its tiles are written either way so the map gives
/// nothing away, and only the rock's breakability follows the flag.
pub(crate) const HAMMER_ROCKS: &[(usize, Pos, u8)] = &[(0, (6, 5), 0x51), (W8_IDX, (3, 37), 0x51)];

/// W8 (Dark World) canoe + extra-path edits, gated behind the `8s are Wild`
/// option.
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
/// `More hammer rocks` option (see [`HAMMER_ROCKS`]) and is placed
/// independently of this flag.
const W8_CANOE_PATH_EDITS: &[(usize, usize, u8)] = &[
    // --- Screen 0: canoe docks + navy approach ---
    (3, 8, 0x4B),
    (3, 10, 0x44),
    (3, 12, 0x44),
    (4, 8, 0x85),
    (4, 10, 0x46),
    (4, 12, 0x46),
    (5, 6, 0x4B),
    (5, 7, 0x8C),
    (5, 8, 0x8D),
    (5, 10, 0x4B),
    (5, 12, 0x4B),
    // --- Screen 2: extra paths ---
    (1, 36, 0x44),
    (1, 37, 0x45),
    (1, 38, 0x45),
    (1, 39, 0x45),
    (1, 40, 0x47),
    (1, 41, 0x45),
    (1, 42, 0x47),
    (1, 43, 0x45),
    (1, 44, 0x47),
    (1, 45, 0x45),
    (1, 46, 0x47),
    (2, 36, 0x46),
    (2, 46, 0x46),
    (3, 36, 0x4A),
    (3, 46, 0x48),
    (4, 46, 0x46),
    (5, 45, 0x45),
    (5, 46, 0x4A),
];

/// W1 shortcut edits — the rock's BEHAVIOUR is gated by `More hammer rocks`
/// (see [`apply_w1_shortcut`]), but the tiles are written either way.
///
/// Vanilla W1 is one long lap: the middle chain (row 4) and the bottom-right
/// chain (row 6) only meet by walking all the way around the bottom-left —
/// (4,4)→(6,4)→(8,4)→(8,6)→(8,8)→(6,8). This drops a single hammer-breakable
/// vertical rock at (5,8), joining the two chains directly:
///
/// ```text
///   (4,8) spade  --rock(5,8)-->  (6,8)
/// ```
///
/// The rock (`0x52`, opening to `0x46`) rather than a bare path is deliberate.
/// As a plain path this link is free, and it lands three hops from the start
/// and two from the airship, so it hands out a near-straight start→goal line:
/// W1's cheapest route fell to a C1 min of 8 with 7.2% of seeds under 12,
/// where no other world ever prices below 12. `COST_ROCK` is 8 points, so
/// gating it puts that back while keeping the second route.
///
/// The plain walk treats `0x52` as a wall (it is in neither `VALID_HORZ` nor
/// `VALID_VERT`), so base connectivity is unchanged from vanilla and the
/// connectivity phase never leans on this link. Only `analyze_route_choice`
/// opens rocks, prices them, and keeps them in a route's identity.
///
/// `0x4A` at (6,8) is the both-directions blank node — it draws the path stub
/// running up into the rock, so the shortcut reads as a shortcut. Vanilla's
/// `0x47` is the horizontal-only variant.
///
/// Rocks are also cleared by `Map_Reload_with_Completions` when the cell's
/// completion bit is set (`Map_Removable_Tiles` → `Map_RemoveTo_Tiles`,
/// `TILE_ROCKBREAKV` → `TILE_VERTPATH`), not by the hammer alone — which is
/// how the optional W1 (6,5) rock ends up clearing itself. Row 5 is safe on
/// two counts: it owns its completion bit outright (only the `$01` bit does
/// the extra-row pass at `PRG012_A55C`, which is the rows 7/8 sharing), and a
/// rock is not a completable tile, so nothing sets that bit in the first
/// place. `0x52` is also absent from `LOCKABLE_TILES`, so the builder can
/// never drop a fortress lock here and clobber the rock.
///
/// Node parity is load-bearing. `Map_CheckDoMove` (PRG010) validates only the
/// ADJACENT tile and then stores a hardcoded `#32` into `World_Map_Move` — two
/// tiles, always — so a new node has to sit an EVEN number of tiles from the
/// node it hangs off, with exactly one path tile between. Drawing a node
/// adjacent to its anchor with two stacked path tiles below looks right in a
/// tile editor and is completely dead in game.
const W1_SHORTCUT_ROCK: (usize, usize) = (5, 8);
const W1_SHORTCUT_STUB: (usize, usize, u8) = (6, 8, 0x4A);

/// Write the W1 shortcut (see [`W1_SHORTCUT_ROCK`] and [`W1_SHORTCUT_STUB`]). The tiles go down
/// unconditionally; `breakable` only decides whether the rock can be opened.
///
/// `0x52` (breakable, opens to `0x46`) and `0x53` (permanent wall) are
/// PIXEL-IDENTICAL — same CHR quad `0C/0D/0E/0F`, same palette page 1 — and
/// differ only in whether `Map_Removable_Tiles` lists them. So the map looks
/// the same either way and a player cannot tell from looking whether the
/// option came up on. That matters because `More hammer rocks` is a `Tri`
/// flag: under `Maybe` the seed decides, and a visible difference would leak
/// the roll before they ever swing a hammer.
///
/// The `0x4A` stub at (6,8) is part of the disguise, not the shortcut — it
/// draws the path running up into the rock in both cases. It stays a valid
/// blank node either way, so pickup preserves it (`blank_tile_for` returns any
/// `VALID_BLANK_TILES` tile unchanged).
///
/// Applied before the overworld builder reads the map, so the builder prices
/// a breakable rock into its route analysis — and, when it isn't breakable,
/// sees a plain wall.
fn apply_w1_shortcut(maps: &mut [WorldMap], breakable: bool) {
    let grid = &mut maps[0].grid;
    let (rock_row, rock_col) = W1_SHORTCUT_ROCK;
    grid.set(rock_row, rock_col, if breakable { 0x52 } else { 0x53 });
    let (stub_row, stub_col, stub) = W1_SHORTCUT_STUB;
    grid.set(stub_row, stub_col, stub);
}

/// W8 (Dark World) screen-3 water decor, always applied: the row above the
/// bridge approach becomes open water so the corridor reads as a chain of
/// spans rather than a solid path.
const W8_WATER_EDITS: &[(usize, usize, u8)] = &[
    (4, 51, 0x99),
    (4, 52, 0xA2),
    (4, 53, 0x83),
    (4, 54, 0xA2),
    (4, 55, 0x83),
    (4, 56, 0xA2),
    (4, 57, 0x83),
    (4, 58, 0xA2),
    (4, 59, 0x9A),
];

/// The W8 bridge approach to Bowser's castle: five spans on screen 3, row 5,
/// alternating with the nodes between them. Each is an ordinary lockable path
/// tile, so a lock landing on one shows as a water gap (`gap_tile_for`:
/// `0xB3 -> 0x9D`) that its fortress rebuilds — the "bridge out" the builder
/// deals per seed (`crate::randomize::overworld::build::locks`). Single source of truth: the
/// builder reads these positions rather than repeating the coordinates.
///
/// All five are stamped as bridges in every mode. In WORLD-MAZE mode only,
/// the last of them — `rom_data::W8_WAND_GATE_POS` at (5,59) — is held out
/// of the deal by `crate::randomize::overworld::build::locks::free_bridge_spans`, because the
/// maze writes its wand gate over that cell after the build. It stays stamped
/// here either way: the corridor looks the same, and the gate goes down last,
/// over the finished map.
pub(crate) const W8_BRIDGE_ROW: usize = 5;
pub(crate) const W8_BRIDGE_COLS: [usize; 5] = [51, 53, 55, 57, 59];

/// Apply the always-on W8 screen-3 water + bridge approach (see
/// [`W8_WATER_EDITS`] and [`W8_BRIDGE_COLS`]). Independent of the `8s are
/// Wild` option.
fn apply_w8_bridges(maps: &mut [WorldMap]) {
    let grid = &mut maps[W8_IDX].grid;
    for &(row, col, tile) in W8_WATER_EDITS {
        grid.set(row, col, tile);
    }
    for col in W8_BRIDGE_COLS {
        grid.set(W8_BRIDGE_ROW, col, BRIDGE_TILE);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::randomize::rom_data::map_tile_offset;

    /// The drawbridge and rock edits used to be raw file offsets. Pin the
    /// conversion to positions against those offsets, so a slip in the
    /// arithmetic cannot move an edit to a neighbouring cell.
    #[test]
    fn positions_match_the_old_file_offsets() {
        let old: &[(usize, Pos, usize)] = &[
            (2, W3_DRAWBRIDGE_TILES[0].1, 0x18777),
            (2, W3_DRAWBRIDGE_TILES[1].1, 0x18779),
            (2, W3_DRAWBRIDGE_TILES[2].1, 0x1880C),
            (2, W3_DRAWBRIDGE_TILES[3].1, 0x188F3),
            (1, REMOVED_ROCKS[0].1, 0x186E0),
            (2, REMOVED_ROCKS[1].1, 0x187DB),
            (3, REMOVED_ROCKS[2].1, 0x18A16),
            (0, HAMMER_ROCKS[0].1, 0x1861F),
        ];
        for &(world, (row, col), offset) in old {
            assert_eq!(map_tile_offset(world, row, col), offset, "W{} ({row},{col})", world + 1);
        }
    }
}
