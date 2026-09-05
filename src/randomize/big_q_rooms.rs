//! Big [?] Block bonus-room shuffle.
//!
//! Each of the 11 levels with a Big [?] pipe draws a room from a pool of 19 —
//! the 11 vanilla rooms plus the 8 in "Unused Level 5", a fully unreferenced
//! test level (TCRF) whose eight one-screen rooms are otherwise dead data.
//!
//! Off by default, behind `shuffle_big_q_rooms`. With it off nothing here runs
//! and no RNG is drawn — [`vanilla_assignments`] just reports where the eleven
//! pipes already lead, so 7-F1's flight-suit protection still knows which room
//! to force.
//!
//! # How a room is assigned
//!
//! `qol::big_q` replaced vanilla's `LDY World_Num` with a lookup keyed on level
//! identity, and that lookup now also seeds the engine's spawn slots. So moving
//! a host to a new room is three table writes and no layout edits:
//!
//! * `BQ_ROOM[row]`   — which bonus AREA to open.
//! * `BQ_ARRIVE[row]` — where to land inside it; the byte2 screen nibble is
//!   what picks the room within the area.
//! * `BQ_RETURN[row]` — where to put the player back in the host, which is a
//!   property of the host and so never changes.
//!
//! Rooms in Unused Level 5 open through bonus-area slot 0, which is vanilla's
//! empty World 1 area and is referenced by nothing.
//!
//! # The one constraint
//!
//! `BigQBlock_GotIt` is an 8-bit mask indexed by the block's *screen* and
//! cleared on world entry, so two hosts in the same world holding rooms with
//! the same screen number would share one "already opened" bit. Rooms are drawn
//! per world with the screens already taken excluded.

use rand::Rng;
use rand::seq::SliceRandom;

use crate::randomize::qol::big_q;
use crate::randomize::rom_data;
use crate::rom::Rom;

/// `OBJ_BIGQBLOCK_TANOOKI` — the flight suit 7-F1 needs whichever room it draws.
pub(crate) const BIGQBLOCK_TANOOKI: u8 = 0x98;

/// A room a host can be sent to.
#[derive(Clone, Copy)]
struct Room {
    /// Index into the `LevelJctBQ_*` tables. 0 is the repurposed slot that
    /// points at Unused Level 5.
    area: u8,
    /// Screen within the area — also the `BigQBlock_GotIt` bit.
    screen: u8,
    /// Spawn bytes `(byte1, byte2)` that land the player at the room's pipe.
    arrive: (u8, u8),
    /// Unused Level 5 rooms need their area slot pointed at the level.
    unused5: bool,
}

const fn room(area: u8, screen: u8, arrive: (u8, u8), unused5: bool) -> Room {
    Room { area, screen, arrive, unused5 }
}

/// The 11 vanilla rooms, each with the arrival its vanilla host uses. The byte2
/// screen nibble matches the room's screen, which is what makes these reusable
/// by any host.
const VANILLA_ROOMS: [Room; 11] = [
    room(2, 4, (0x02, 0x14), false), // BigQ3 s4  3-Up
    room(2, 5, (0x02, 0x15), false), // BigQ3 s5  Frog
    room(3, 2, (0x52, 0x22), false), // BigQ4 s2  3-Up
    room(4, 3, (0x02, 0x73), false), // BigQ5 s3  3-Up
    room(4, 7, (0x02, 0x17), false), // BigQ5 s7  Tanooki
    room(5, 5, (0x52, 0x25), false), // BigQ6 s5  Tanooki
    room(5, 3, (0x02, 0x83), false), // BigQ6 s3  3-Up
    room(5, 6, (0x12, 0xD6), false), // BigQ6 s6  Hammer
    room(6, 6, (0x02, 0x16), false), // BigQ7 s6  Tanooki
    room(6, 4, (0x52, 0x14), false), // BigQ7 s4  Hammer
    room(7, 4, (0x02, 0xD4), false), // BigQ8 s4  3-Up
];

/// Unused Level 5's eight rooms, one per screen, every block a Tanooki Suit.
///
/// Arrivals aim at each room's ceiling pipe at a Y index *inside* it, so the
/// player falls out of the mouth the way vanilla does. Dir 2 throughout,
/// matching vanilla's Big [?] arrivals.
///
/// The Y index is into `LevelJct_YLHStarts`, which has only eight entries —
/// rows 0, 4, 7, 11, 15, 20, 23, 24 — so a landing spot cannot be nudged by a
/// row or two. That coarseness is why screens 6 and 7 took a playtest round:
/// see their comment below. Screen 2 needed a change to the room instead of the
/// arrival, which is what `SCREEN2_NOTES` is.
const UNUSED5_ROOMS: [Room; 8] = [
    room(0, 0, (0x02, 0x20), true), // ceiling pipe col 2, rows 0-1
    room(0, 1, (0x42, 0x11), true), // ceiling pipe col 1, rows 12-16
    // Screen 2's shaft used to be a one-way trip past the block; its two coins
    // are note blocks now, so the floor is no longer a dead end. See
    // `SCREEN2_NOTES`.
    room(0, 2, (0x02, 0x22), true), // ceiling pipe col 2, rows 0-1
    room(0, 3, (0x52, 0x13), true), // ceiling pipe col 1, rows 16-21
    room(0, 4, (0x52, 0x24), true), // ceiling pipe col 2, rows 16-21
    room(0, 5, (0x02, 0x75), true), // ceiling pipe col 7, rows 0-2
    // Screens 6 and 7 have no ceiling pipe, so there is no mouth to fall out
    // of and the aim had to be found by playtesting (2026-08-28). Both were
    // originally Y index 6 — row 23, the floor pipe's own row — which put the
    // player in the pipe or past the end of the floor and killed him.
    room(0, 6, (0x52, 0x36), true), // col 3 row 20: lands beside the floor pipe
    room(0, 7, (0x32, 0x27), true), // col 2 row 11: falls into the floor pipe
];

/// A level with a Big [?] pipe: its row in the lookup table, its entry
/// `obj_ptr` (used to find which world it ended up in), and the mirror row that
/// must move with it — the lobby-shuffle path into the same level.
struct Host {
    name: &'static str,
    row: usize,
    mirror: Option<usize>,
    obj_ptr: u16,
}

const fn host(name: &'static str, row: usize, mirror: Option<usize>, obj_ptr: u16) -> Host {
    Host { name, row, mirror, obj_ptr }
}

const HOSTS: [Host; 11] = [
    host("3-5", 0, None, 0xCDEB),
    host("3-9", 1, None, 0xC38F),
    host("4-F2", 2, None, 0xD508),
    host("5-2", 3, Some(11), 0xC8BE),
    host("5-5", 4, None, 0xCB0A),
    host("6-3", 5, None, 0xCA8E),
    host("6-9", 6, Some(12), 0xCD2D),
    host("6-10", 7, None, 0xCCE8),
    host("7-F1", 8, None, 0xD4E4),
    host("7-8", 9, None, 0xC32D),
    host("8-1", 10, None, 0xC424),
];

/// What one host ended up with, for the write log and the 7-F1 protection.
pub(crate) struct Assignment {
    pub name: &'static str,
    pub area: u8,
    pub screen: u8,
}

/// Which world a host's level sits in after the overworld builder has run, by
/// looking its `obj_ptr` up in the world pointer tables. `None` means the level
/// is not placed on any map, in which case its room can never be opened and the
/// screen rule does not apply to it.
fn world_of(rom: &Rom, obj_ptr: u16) -> Option<usize> {
    let (lo, hi) = (obj_ptr as u8, (obj_ptr >> 8) as u8);
    for (wi, world) in rom_data::WORLDS.iter().enumerate() {
        for idx in 0..world.entry_count {
            let e = rom_data::read_entry(rom, world, idx);
            if e.obj_lo == lo && e.obj_hi == hi {
                return Some(wi);
            }
        }
    }
    None
}

/// What every host opens when the shuffle is off — read back out of the vanilla
/// lookup tables rather than restated, so there is one source of truth for
/// "7-F1 opens BigQ7 s6".
///
/// The caller still needs this: the block *contents* roll exempts nothing, so
/// 7-F1's room has to be forced to a flight suit whether or not its room moved.
pub(crate) fn vanilla_assignments() -> Vec<Assignment> {
    HOSTS
        .iter()
        .map(|h| Assignment {
            name: h.name,
            area: big_q::BQ_ROOM[h.row],
            screen: big_q::BQ_ARRIVE[h.row].1 & 0x0F,
        })
        .collect()
}

/// Draw a room for every Big [?] host and write the lookup tables.
///
/// Runs after `qol::fix_big_q_block_rooms` (which writes the routine this fills
/// in) and after the overworld builder (which decides what world each host is
/// in). Returns the assignments so callers can protect 7-F1's block.
pub(crate) fn shuffle<R: Rng>(rom: &mut Rom, rng: &mut R) -> Vec<Assignment> {
    let mut rooms: Vec<Room> = VANILLA_ROOMS.iter().chain(UNUSED5_ROOMS.iter()).copied().collect();
    rooms.shuffle(rng);

    let mut order: Vec<usize> = (0..HOSTS.len()).collect();
    order.shuffle(rng);

    let mut screens_used: [Vec<u8>; 8] = Default::default();
    let mut rooms_out: Vec<Option<Room>> = vec![None; HOSTS.len()];
    let mut assignments = Vec::new();
    let mut any_unused5 = false;

    for hi in order {
        let h = &HOSTS[hi];
        let world = world_of(rom, h.obj_ptr);
        // Skip screens already spoken for in this world — they would share one
        // BigQBlock_GotIt bit.
        let taken: &[u8] = world.map(|w| screens_used[w].as_slice()).unwrap_or(&[]);
        let Some(pos) = rooms.iter().position(|r| !taken.contains(&r.screen)) else {
            continue; // nothing legal left; the host keeps whatever it has
        };
        let r = rooms.remove(pos);
        if let Some(w) = world {
            screens_used[w].push(r.screen);
        }
        any_unused5 |= r.unused5;
        rooms_out[hi] = Some(r);
        assignments.push(Assignment { name: h.name, area: r.area, screen: r.screen });
    }

    // Start from vanilla so any host that drew nothing keeps working.
    let mut area_of = big_q::BQ_ROOM;
    let mut arrive = big_q::BQ_ARRIVE;
    let ret = big_q::BQ_RETURN;
    for (hi, drawn) in rooms_out.iter().enumerate() {
        let (Some(r), h) = (drawn, &HOSTS[hi]) else { continue };
        for row in [Some(h.row), h.mirror].into_iter().flatten() {
            area_of[row] = r.area;
            arrive[row] = r.arrive;
        }
    }
    big_q::write_room_tables(rom, &area_of, &arrive, &ret);

    if any_unused5 {
        point_slot0_at_unused5(rom);
    }
    assignments
}

/// Repoint bonus-area slot 0 — vanilla's empty World 1 area, referenced by
/// nothing — at Unused Level 5, and give it a real BG palette. Its own header
/// carries index 6, the placeholder every unused fortress-tileset level uses,
/// which draws the rooms in black and white.
const UNUSED5_BGPAL: u8 = 4;

fn point_slot0_at_unused5(rom: &mut Rom) {
    rom.write_range(rom_data::BIG_Q_AREA_LAYOUTS, &rom_data::UNUSED5_LAYOUT_PTR.to_le_bytes());
    rom.write_range(rom_data::BIG_Q_AREA_OBJECTS, &rom_data::UNUSED5_OBJECT_PTR.to_le_bytes());
    rom.write_byte(rom_data::BIG_Q_AREA_TILESETS, rom_data::UNUSED5_TILESET);

    let hdr =
        rom_data::prg_bank_cpu_to_file(rom_data::UNUSED5_LAYOUT_BANK, rom_data::UNUSED5_LAYOUT_PTR);
    let byte5 = rom.read_byte(hdr + 5);
    rom.write_byte(hdr + 5, (byte5 & 0xF8) | UNUSED5_BGPAL);

    write_screen2_notes(rom, SCREEN2_NOTES);
}

/// Screen 2's two `Coin` commands, which [`write_screen2_notes`] retypes.
///
/// Screen 2 is the one room a player can be locked out of. You arrive through
/// its ceiling pipe at column 2, and the shaft below (columns 1-4) runs unbroken
/// to the floor at row 25 while the block sits at row 10, column 10 — reachable
/// only by drifting right through the rows 8-10 gap on the way down. Ride the
/// shaft to the floor instead and there is nothing to climb: the exit pipe is
/// still reachable, so the player leaves having spent the room for nothing.
///
/// Both coins sit in that shaft, which makes them the two tiles worth spending
/// on a way back up.
const SCREEN2_COINS: [(usize, [u8; 3]); 2] =
    [(0x2B7C0, [0x2A, 0x22, 0x80]), (0x2B7C3, [0x2B, 0x21, 0x80])];

/// Where the note blocks go, as `(row, column)`, tuned by playtest 2026-08-30.
///
/// Row 20 column 1 stands over the low floor at the bottom of the shaft, within
/// a running jump of it and of the row 23 platform to its right. Row 15 column 4
/// is one row under the top of the big columns 5-12 platform, so the bounce off
/// the first lands beside it and the last move is a step up rather than a climb;
/// from that platform the block is two tiles over the player's head.
///
/// Earlier positions were higher (19/12, then 19/13, then 20/14) and all read as
/// out of reach in play — the bounce is shorter than the geometry suggests.
const SCREEN2_NOTES: [(u8, u8); 2] = [(20, 1), (15, 4)];

/// Retype screen 2's two coins as note blocks at `at`.
///
/// A note block (`TILEA_NOTE`, $2E) is solid in this tileset — solidity is
/// `tile >= Tile_AttrTable[quadrant]`, and tileset 2's quadrant-0 root is $11 —
/// and `Level_ActionTiles` bounces the player off it from above regardless of
/// tileset, so nothing here needs a patch to go with it. Retyping is a one-
/// nibble edit: byte2's high nibble picks the block out of `LoadLevel_Blocks`,
/// $8 = coin, $6 = note block. Both commands are rewritten whole rather than
/// patched in place, because the position lives in bytes 0 and 1.
///
/// Costs no ROM space: these are three-byte commands overwritten in place.
fn write_screen2_notes(rom: &mut Rom, at: [(u8, u8); 2]) {
    for (i, &(off, _)) in SCREEN2_COINS.iter().enumerate() {
        let (row, col) = at[i];
        // byte0: group 1, bit 4 selects the lower half of screen memory and the
        // low nibble the row within that half. byte1: screen 2, column.
        // byte2: $60 = note block, run length 1.
        rom.write_range(off, &[0x20 | (row & 0x10) | (row & 0x0F), 0x20 | col, 0x60]);
    }
}

/// The vanilla bytes at [`SCREEN2_COINS`], for callers that verify their base
/// before writing over it.
///
/// Native-only, like its one caller `testrom` — the web build never assembles a
/// ROM from an unverified base, and an unconditional accessor trips CI's
/// wasm32 `-D warnings` pass as dead code.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn screen2_coins() -> [(usize, [u8; 3]); 2] {
    SCREEN2_COINS
}

/// Where [`point_slot0_at_unused5`] puts the note blocks, so `testrom` can
/// reproduce the shipped layout rather than a variant of it. Native-only for
/// the same reason as [`screen2_coins`].
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn screen2_notes() -> [(u8, u8); 2] {
    SCREEN2_NOTES
}

/// Lay the note blocks at an arbitrary `at`, for `testrom`'s position override.
/// The shipped positions go in through [`point_slot0_at_unused5`].
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn place_screen2_notes(rom: &mut Rom, at: [(u8, u8); 2]) {
    write_screen2_notes(rom, at);
}

/// File offset of the Big [?] Block object entry in a room, or `None` if the
/// area holds no block on that screen.
pub(crate) fn block_offset(rom: &Rom, area: u8, screen: u8) -> Option<usize> {
    let ptr_off = rom_data::BIG_Q_AREA_OBJECTS + area as usize * 2;
    let obj_ptr = u16::from_le_bytes([rom.read_byte(ptr_off), rom.read_byte(ptr_off + 1)]);
    // Object streams open with a 1-byte prefix, then 3-byte entries, then $FF.
    let mut p = rom_data::enemy_ptr_to_file_offset(obj_ptr) + 1;
    while rom.read_byte(p) != 0xFF {
        let (id, x) = (rom.read_byte(p), rom.read_byte(p + 1));
        if (0x94..=0x9A).contains(&id) && x >> 4 == screen {
            return Some(p);
        }
        p += 3;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_room_arrival_names_its_own_screen() {
        // byte2 is (col << 4) | screen — the low nibble is what picks the room,
        // so a row whose arrival disagrees with its screen would open a
        // different room than the table claims.
        for r in VANILLA_ROOMS.iter().chain(UNUSED5_ROOMS.iter()) {
            assert_eq!(r.arrive.1 & 0x0F, r.screen, "area {} screen {}", r.area, r.screen);
        }
    }

    #[test]
    fn vanilla_rooms_are_distinct() {
        let mut seen = Vec::new();
        for r in VANILLA_ROOMS.iter().chain(UNUSED5_ROOMS.iter()) {
            assert!(!seen.contains(&(r.area, r.screen)), "duplicate room");
            seen.push((r.area, r.screen));
        }
        assert_eq!(seen.len(), 19);
    }

    #[test]
    fn host_rows_match_the_lookup_table() {
        // Each host's row must key on that host's obj_ptr, or it would rewrite
        // some other level's room.
        for h in &HOSTS {
            let hi = big_q::BQ_OBJ_HI[h.row];
            let lo = big_q::BQ_OBJ_LO[h.row];
            assert_eq!(u16::from_le_bytes([lo, hi]), h.obj_ptr, "{}", h.name);
        }
    }

    #[test]
    fn mirror_rows_are_the_documented_pairs() {
        let pairs: Vec<(usize, usize)> =
            HOSTS.iter().filter_map(|h| h.mirror.map(|m| (h.row, m))).collect();
        assert_eq!(pairs, big_q::BQ_MIRROR_ROWS.to_vec());
    }

    /// The reason the note blocks exist is that a player who lands on screen 2's
    /// floor is otherwise stuck, so the write has to be wired into the one
    /// function that runs when a host actually draws an Unused Level 5 room.
    /// Deleting that call would leave every other test here passing.
    #[test]
    fn opening_unused5_lays_the_screen_2_note_blocks() {
        let Some(bytes) = std::fs::read("roms/Super Mario Bros. 3 (USA) (Rev 1).nes").ok() else {
            return;
        };
        let mut rom = Rom::from_bytes_lax(&bytes, true).unwrap();

        // Vanilla first: a moved offset would silently retype another room.
        for (off, expect) in SCREEN2_COINS {
            assert_eq!(rom.read_range(off, 3), &expect, "screen 2 coin at {off:#07X}");
        }

        point_slot0_at_unused5(&mut rom);

        for (i, (off, _)) in SCREEN2_COINS.into_iter().enumerate() {
            let (row, col) = SCREEN2_NOTES[i];
            let cmd = rom.read_range(off, 3);
            // Group 1 with byte2 $60 is LoadLevel_BlockRun's note block, run of
            // one; a different group decodes as some other generator entirely.
            assert_eq!(cmd[0] >> 5, 1, "generator group at {off:#07X}");
            assert_eq!(cmd[2], 0x60, "block type at {off:#07X}");
            // Round-trip the position back out of bytes 0 and 1.
            assert_eq!((cmd[0] & 0x0F) + 16 * ((cmd[0] >> 4) & 1), row, "row at {off:#07X}");
            assert_eq!(cmd[1] >> 4, 2, "screen at {off:#07X}");
            assert_eq!(cmd[1] & 0x0F, col, "column at {off:#07X}");
        }
    }

    /// Both note blocks have to sit in the shaft the player falls down —
    /// columns 1-4, below the rows 8-10 gap — or they are decoration.
    #[test]
    fn the_note_blocks_are_in_the_shaft() {
        for (row, col) in SCREEN2_NOTES {
            assert!((1..=4).contains(&col), "column {col} is outside the shaft");
            assert!((11..=24).contains(&row), "row {row} is not between the gap and the floor");
        }
    }
}
