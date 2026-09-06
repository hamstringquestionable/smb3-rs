//! **POC** — per-world map progress that survives leaving and re-entering a
//! world.
//!
//! The world-maze design needs one thing vanilla has no concept of: a world you
//! can come back to. Vanilla never returns you to a world, so it keeps exactly
//! one world's completion state and wipes it on every transition —
//! `Map_Completions` is 128 bytes at `$7D00`, split Mario / Luigi, "allows a
//! MAX of 4 map screens" per the disassembly, and `PRG030_84A0` ("initialize
//! the world map") opens by zeroing all 128 bytes:
//!
//! ```text
//! $84CD   A0 7F       LDY #$7F
//!         A9 00       LDA #$00
//!         99 00 7D    STA Map_Completions,Y
//!         88          DEY
//!         10 FA       BPL -6
//! ```
//!
//! Ten bytes, three whole instructions, nothing branches into the middle of it.
//! Replacing them is the entire hook — and by the time they run, `$84A0` has
//! already mapped PRG010 into `$C000` (its first act is `LDA #10 / STA
//! PAGE_C000`), so the replacement can live in PRG010's free space with no
//! trampoline and no always-mapped-bank scarcity.
//!
//! **What this POC does and does not prove.** It answers one question: with the
//! wipe replaced by a save/restore, does the map render correctly on return?
//! `$84A0` calls `Map_Reload_with_Completions` well after the wipe, so if the
//! array holds the right bits by then the tiles should come back beaten with no
//! further work. That is the hypothesis under test.
//!
//! **Luigi's half of `Map_Completions` is not spare storage.** The first cut of
//! this POC parked the other world at `$7D40` on the theory that one-player mode
//! never touches it. It does. Three sites mirror a completion into the *other*
//! player's array unconditionally, because a permanent map alteration has to
//! survive a game over:
//!
//! - `PRG011_BA7C`, level clear — "Fortress only... mark complete on both
//!   Players (so it remains after Game Over)"
//! - `MO_DoFortressFX` (PRG010) — "Mark lock busted / bridge built (Luigi)",
//!   `Map_Completions+$40,Y`
//! - `Map_SetCompletion_By_Poof` (PRG026) — "Rock removal sets completion bit
//!   for BOTH Players!"
//!
//! So clearing a fortress, busting a lock or smashing a rock in the live world
//! stamped the parked world's bank, and the next swap carried that stamp onto a
//! map it had nothing to do with — a completed tile appearing in World 2 at the
//! column World 1's was on. The parked bank now lives in genuinely unused SRAM
//! instead, which has the side benefit of proving that region is really free.
//!
//! **And the mirror is not private to Luigi either — the map reload applies
//! both halves.** `Map_Reload_with_Completions` loops to `CMP #$80`, all 128
//! bytes, folding the column index with `AND #$30`, which aliases Luigi's bytes
//! onto the same four screens as Mario's. Its own comment says so:
//!
//! ```text
//! ; Note: Loop goes through both Players sets of completion bits, but
//! ; this AND will basically cause 2 passes across the map...
//! ```
//!
//! So banking only Mario's half still leaked: a fortress cleared in World 1 left
//! its mirrored bit at `$7D40 + col`, and World 2's reload applied it to
//! whatever sat at that column — a pyramid, in the case this was found from.
//! Both halves are banked now, which also leaves the live world's game-over
//! semantics intact: `PRG030_9314` ANDs the two, and that is exactly how vanilla
//! keeps forts and locks broken while wiping plain level clears.
//!
//! It deliberately does **not** solve storage for eight worlds: 8 x 128 = 1024
//! bytes against 384 of declared-unused SRAM, so the real version has to store
//! bits per completable cell against a randomizer-emitted table rather than raw
//! columns — two bits, since the mirror is a real distinction the engine makes
//! and not a copy. At ~40 cells per world that is comfortable. Arithmetic, and
//! not what this POC is for.
//!
//! Also knowingly unhandled here: the player respawns at the world's start tile
//! rather than where they left (a portal aims its own arrival, but leaving a
//! world any other way does not remember where you stood), and the per-world
//! flags `$84A0` resets (`Map_Anchored`, `Map_WhiteHouse`, `Map_CoinShip`,
//! `Map_Got13Warp`) are not banked.
//!
//! Vanilla's own two jumps into `$84A0` — the airship and the warp zone — used
//! to be unhandled too, because the pack hook fired on a flag only our own code
//! raised. Both are covered now: the hooks compare `World_Num` against
//! `LIVE_WORLD`, which every path sets, and `completion_bits`' new-game signal
//! is what keeps a fresh game from looking like a transition.

use crate::rom::Rom;

use super::completion_bits;
use super::pipe_helpers;
#[cfg(test)]
use super::rom_data::NMI_SAFE_MAX;
use super::rom_data::{
    FS_PAD_ENTER, FS_PORTAL_ARRIVAL, FS_RESTORE_ARRIVAL, PLAYER_CURRENT, TILE_TELEPAD,
    WORLD_MAP_INIT_CPU, WORLD_MAP_TILE, WORLD_MAP_X, WORLD_MAP_XHI, WORLD_MAP_Y, WORLD_NUM,
    prg010_file_to_cpu, prg011_file_to_cpu,
};

// PRG010 is mapped at $C000 whenever this runs — `$84A0` maps it itself — the
// same bank as the other map patches (`map_warp.rs`, `canoe_summon.rs`).
const RESTORE_ARRIVAL_CPU: u16 = prg010_file_to_cpu(FS_RESTORE_ARRIVAL);
// PRG011 is mapped at $A000 for the whole map init, so the stash and its
// table live there: PRG010 has no run left that holds them.
const STASH_ARRIVAL_CPU: u16 = prg011_file_to_cpu(FS_PORTAL_ARRIVAL);
const PAD_ENTER_CPU: u16 = prg011_file_to_cpu(FS_PAD_ENTER);

// --- Engine symbols -----------------------------------------------------
//
// `World_Num`, `Player_Current`, `World_Map_Tile`, the three live position
// bytes and `$84A0` are all `rom_data::engine`'s; only the addresses this
// module *chose* are declared here.

/// Where the pipeway's arrival coordinates wait out `Map_Init`.
///
/// Six bytes in the `$7A73` run, past the stencil scratch and its counters.
/// They used to sit at `$79D7`, inside what is now
/// [`completion_bits`]' packed region.
const ARRIVAL_Y: u16 = 0x7AB6;
const ARRIVAL_XHI: u16 = 0x7AB7;
const ARRIVAL_X: u16 = 0x7AB8;
const ARRIVAL_SCRL: u16 = 0x7AB9;
const ARRIVAL_SCRH: u16 = 0x7ABA;
const ARRIVAL_FLAG: u16 = 0x7ABB;

/// The player's map position, as `Map_Init` writes it (PRG011). Every one of
/// these is set from `Map_Y_Starts` and friends, so every one has to be
/// rewritten to land somewhere else:
///
/// ```text
/// LDA Map_Y_Starts,Y
/// STA Map_Entered_Y,X / STA Map_Previous_Y,X
/// ... STA Map_Entered_X,X / STA Map_Previous_X,X
/// ... STA Map_Entered_XHi,X / STA Map_Previous_XHi,X
/// ... STA Map_Prev_XOff2,X / STA Map_Prev_XHi2,X
/// ... STA Map_Prev_XOff,X  / STA Map_Prev_XHi,X
/// ```
const MAP_ENTERED_Y: u16 = 0x7976;
const MAP_ENTERED_XHI: u16 = 0x7978;
const MAP_ENTERED_X: u16 = 0x797A;
const MAP_PREVIOUS_Y: u16 = 0x797E;
const MAP_PREVIOUS_XHI: u16 = 0x7980;
const MAP_PREVIOUS_X: u16 = 0x7982;
const MAP_PREV_XOFF2: u16 = 0x7986;
const MAP_PREV_XHI2: u16 = 0x7988;
const MAP_PREV_XOFF: u16 = 0x0722;
const MAP_PREV_XHI: u16 = 0x0724;

/// The 10-byte `Map_Completions` wipe inside `PRG030_84A0` (CPU `$84CD`).
const WIPE_OFFSET: usize = 0x3C4DD;
/// Length of that run. `completion_bits` writes its own call into the first
/// three bytes and pads the rest; the arrival restore goes into the padding, so
/// only the tests here need the length.
#[cfg(test)]
const WIPE_LEN: usize = 10;

// --- Routines -----------------------------------------------------------

/// Put the player where the portal aimed them, after `Map_Init` has had its say.
///
/// `Map_Init` writes the destination world's start into ten variables, so
/// landing anywhere else means rewriting all ten — five values, each in two
/// places. The values themselves are whatever `ObjNorm_PipewayCtlr` computed
/// on the way out of the transit room, copied byte for byte: those bytes are
/// what vanilla uses to place you on a same-world pipe return, so their
/// encoding is right by construction and needs no arithmetic here.
///
/// A no-op unless a portal set the flag, which is why this can sit
/// unconditionally at the end of the completion swap.
#[rustfmt::skip]
const RESTORE_ARRIVAL: [u8; 56] = [
    0xAD, ARRIVAL_FLAG as u8, (ARRIVAL_FLAG >> 8) as u8,        //  0: LDA ARRIVAL_FLAG
    0xF0, 0x32,                                                 //  3: BEQ +50 → done
    0xA9, 0x00,                                                 //  5: LDA #$00
    0x8D, ARRIVAL_FLAG as u8, (ARRIVAL_FLAG >> 8) as u8,        //  7: STA ARRIVAL_FLAG

    0xAD, ARRIVAL_Y as u8, (ARRIVAL_Y >> 8) as u8,              // 10: LDA ARRIVAL_Y
    0x8D, MAP_ENTERED_Y as u8, (MAP_ENTERED_Y >> 8) as u8,      // 13: STA Map_Entered_Y
    0x8D, MAP_PREVIOUS_Y as u8, (MAP_PREVIOUS_Y >> 8) as u8,    // 16: STA Map_Previous_Y

    0xAD, ARRIVAL_X as u8, (ARRIVAL_X >> 8) as u8,              // 19: LDA ARRIVAL_X
    0x8D, MAP_ENTERED_X as u8, (MAP_ENTERED_X >> 8) as u8,      // 22: STA Map_Entered_X
    0x8D, MAP_PREVIOUS_X as u8, (MAP_PREVIOUS_X >> 8) as u8,    // 25: STA Map_Previous_X

    0xAD, ARRIVAL_XHI as u8, (ARRIVAL_XHI >> 8) as u8,          // 28: LDA ARRIVAL_XHI
    0x8D, MAP_ENTERED_XHI as u8, (MAP_ENTERED_XHI >> 8) as u8,  // 31: STA Map_Entered_XHi
    0x8D, MAP_PREVIOUS_XHI as u8,
          (MAP_PREVIOUS_XHI >> 8) as u8,                        // 34: STA Map_Previous_XHi

    0xAD, ARRIVAL_SCRL as u8, (ARRIVAL_SCRL >> 8) as u8,        // 37: LDA ARRIVAL_SCRL
    0x8D, MAP_PREV_XOFF as u8, (MAP_PREV_XOFF >> 8) as u8,      // 40: STA Map_Prev_XOff
    0x8D, MAP_PREV_XOFF2 as u8, (MAP_PREV_XOFF2 >> 8) as u8,    // 43: STA Map_Prev_XOff2

    0xAD, ARRIVAL_SCRH as u8, (ARRIVAL_SCRH >> 8) as u8,        // 46: LDA ARRIVAL_SCRH
    0x8D, MAP_PREV_XHI as u8, (MAP_PREV_XHI >> 8) as u8,        // 49: STA Map_Prev_XHi
    0x8D, MAP_PREV_XHI2 as u8, (MAP_PREV_XHI2 >> 8) as u8,      // 52: STA Map_Prev_XHi2

    0x60,                                                       // 55: RTS   ; done
];

// --- Writer -------------------------------------------------------------

/// Install the POC: packed per-world completions and any cross-world telepads.
///
/// The storage itself is [`completion_bits`]; this module contributes the
/// telepads that cross between them. The arrival restore is written into the
/// padding `completion_bits` leaves at the wipe site, so it runs on the same
/// pass and after `Map_Init` has had its say.
pub(crate) fn apply(rom: &mut Rom, telepads: &[Telepad]) {
    // Must come first: it owns the wipe site, and the arrival restore below
    // writes into the padding it leaves.
    completion_bits::apply(rom);

    if telepads.is_empty() {
        return;
    }

    rom.push_tag("world_persist");

    // The arrival path, which only a telepad ever raises the flag for.
    rom.write_range(FS_RESTORE_ARRIVAL, &RESTORE_ARRIVAL);
    // Second half of the displaced wipe: `completion_bits` puts its own call in
    // the first three bytes and NOPs the rest, and the restore goes in the
    // padding. Four bytes stay wasted; reclaiming them means restructuring
    // `$84A0` itself, which is not worth it for four bytes.
    rom.write_range(
        WIPE_OFFSET + 3,
        &[0x20, RESTORE_ARRIVAL_CPU as u8, (RESTORE_ARRIVAL_CPU >> 8) as u8],
    );

    let rows: Vec<(u8, (usize, usize))> =
        telepads.iter().map(|t| (t.dest_world, t.dest_pos)).collect();
    write_arrival_tables(rom, &rows);
    install_map_init_trampoline(rom);
    apply_telepads(rom, telepads);

    rom.pop_tag();
}

// --- The arrival path -------------------------------------------------------
//
// **The `PORTAL_*` names are historical, and the code under them is live.**
// This was the pipe portal: a transit room whose exit put the player in another
// world. Telepads replaced it and nothing writes a pipe portal any more — but a
// pad reaches the transition the portal's way, by leaving an arrival id in
// `Map_Entered_XHi` for [`STASH_ARRIVAL`] to resolve, so the whole arrival path
// below is a pad's path now. The names are kept because six modules outside
// this file spell them.

/// How many arrivals a ROM can hold — so, with pads in pairs, eight pads.
///
/// A ceiling the encoding imposes, not a budget: the arrival id travels in the
/// destination's *screen* nibble, which `ObjNorm_PipewayCtlr` masked with
/// `AND #$0F` on the way into `Map_Entered_XHi`. Sixteen is every value that
/// field can carry, so [`STASH_ARRIVAL`] can index its tables with no bounds
/// check at all. Raising it means widening the id, six arrival tables and three
/// key tables — nine bytes a row in PRG011.
pub(crate) const PORTAL_MAX: usize = 16;

/// Length of [`STASH_ARRIVAL`]'s code, and so the offset of its first table.
pub(crate) const PORTAL_TABLE_OFF: usize = 51;

/// The six per-arrival tables, in the order [`STASH_ARRIVAL`] reads them.
/// Parallel arrays rather than 6-byte rows: indexing a row would cost a
/// multiply, indexing a column costs nothing.
const PORTAL_WORLD_CPU: u16 = STASH_ARRIVAL_CPU + PORTAL_TABLE_OFF as u16;
const PORTAL_Y_CPU: u16 = PORTAL_WORLD_CPU + PORTAL_MAX as u16;
const PORTAL_XHI_CPU: u16 = PORTAL_Y_CPU + PORTAL_MAX as u16;
const PORTAL_X_CPU: u16 = PORTAL_XHI_CPU + PORTAL_MAX as u16;
const PORTAL_SCRL_CPU: u16 = PORTAL_X_CPU + PORTAL_MAX as u16;
const PORTAL_SCRH_CPU: u16 = PORTAL_SCRL_CPU + PORTAL_MAX as u16;

/// Total bytes of table.
const PORTAL_TABLE_LEN: usize = 6 * PORTAL_MAX;

/// `JSR Map_Init` inside `PRG030_84A0` (CPU `$84AD` = file 0x3C4BD).
///
/// The trampoline site for the stash. `Map_Init` is what overwrites the
/// pipeway's arrival coordinates, and by this point `$84A0` has already mapped
/// PRG010 into `$C000` and PRG011 into `$A000` — so the replacement can live in
/// either and still reach `Map_Init` in PRG011.
const MAP_INIT_CALL_OFFSET: usize = 0x3C4BD;
const MAP_INIT_CALL_LEN: usize = 3;

/// `Map_Init` itself (PRG011, CPU `$A1D8`).
const MAP_INIT_CPU: u16 = 0xA1D8;

/// Resolve the arrival id and park its coordinates, just before `Map_Init` runs.
///
/// Replaces `$84A0`'s own `JSR Map_Init` and calls it afterwards. That window
/// is the only one where all three things are true at once: `World_Num` can
/// still be changed before `Map_Init` stamps the world start, PRG011 is banked
/// at `$A000` so the tables below are readable, and the completion swap at
/// `$84CD` has not run yet — it packs `LIVE_WORLD`, not `World_Num`, so
/// changing the latter here cannot make it pack the wrong world.
///
/// The id arrives in `Map_Entered_XHi` — where [`PAD_ENTER`] leaves it, and
/// where the retired pipe portal's `ObjNorm_PipewayCtlr` left the destination's
/// screen nibble. Either way it is four bits, so indexing six 16-byte tables
/// with it needs no bound of its own.
///
/// A no-op unless a telepad set the flag, which is why it can sit on a
/// path every map init takes.
#[rustfmt::skip]
const STASH_ARRIVAL: [u8; PORTAL_TABLE_OFF] = [
    0xAD, ARRIVAL_FLAG as u8, (ARRIVAL_FLAG >> 8) as u8,        //  0: LDA ARRIVAL_FLAG
    0xF0, 0x2B,                                                 //  3: BEQ +43 -> Map_Init
    0xAE, PLAYER_CURRENT as u8,
          (PLAYER_CURRENT >> 8) as u8,                          //  5: LDX Player_Current
    0xBD, MAP_ENTERED_XHI as u8,
          (MAP_ENTERED_XHI >> 8) as u8,                         //  8: LDA Map_Entered_XHi,X
    0xAA,                                                       // 11: TAX          ; portal id

    0xBD, PORTAL_WORLD_CPU as u8, (PORTAL_WORLD_CPU >> 8) as u8, // 12: LDA PORTAL_WORLD,X
    0x8D, WORLD_NUM as u8, (WORLD_NUM >> 8) as u8,              // 15: STA World_Num
    0xBD, PORTAL_Y_CPU as u8, (PORTAL_Y_CPU >> 8) as u8,        // 18: LDA PORTAL_Y,X
    0x8D, ARRIVAL_Y as u8, (ARRIVAL_Y >> 8) as u8,              // 21: STA ARRIVAL_Y
    0xBD, PORTAL_XHI_CPU as u8, (PORTAL_XHI_CPU >> 8) as u8,    // 24: LDA PORTAL_XHI,X
    0x8D, ARRIVAL_XHI as u8, (ARRIVAL_XHI >> 8) as u8,          // 27: STA ARRIVAL_XHI
    0xBD, PORTAL_X_CPU as u8, (PORTAL_X_CPU >> 8) as u8,        // 30: LDA PORTAL_X,X
    0x8D, ARRIVAL_X as u8, (ARRIVAL_X >> 8) as u8,              // 33: STA ARRIVAL_X
    0xBD, PORTAL_SCRL_CPU as u8, (PORTAL_SCRL_CPU >> 8) as u8,  // 36: LDA PORTAL_SCRL,X
    0x8D, ARRIVAL_SCRL as u8, (ARRIVAL_SCRL >> 8) as u8,        // 39: STA ARRIVAL_SCRL
    0xBD, PORTAL_SCRH_CPU as u8, (PORTAL_SCRH_CPU >> 8) as u8,  // 42: LDA PORTAL_SCRH,X
    0x8D, ARRIVAL_SCRH as u8, (ARRIVAL_SCRH >> 8) as u8,        // 45: STA ARRIVAL_SCRH

    0x4C, MAP_INIT_CPU as u8, (MAP_INIT_CPU >> 8) as u8,        // 48: JMP Map_Init
];

/// Write the six arrival tables: one row per arrival id.
///
/// The row says only *where you come out*. Today the only thing that reaches an
/// arrival is a telepad — pad `n`'s key row and arrival row are both row `n` —
/// but the table knows nothing about pads, which is why the retired pipe portal
/// could share it and why anything else could.
fn write_arrival_tables(rom: &mut Rom, rows: &[(u8, (usize, usize))]) {
    assert!(
        rows.len() <= PORTAL_MAX,
        "{} arrivals: the id rides in a nibble, so {PORTAL_MAX} is the ceiling",
        rows.len()
    );

    rom.write_range(FS_PORTAL_ARRIVAL, &STASH_ARRIVAL);

    // Unused ids are zeroed rather than left as $FF filler. Nothing can carry
    // one, but a table that reads as "world 255" if anything ever does is a
    // worse failure than one that reads as World 1.
    let mut tables = [0u8; PORTAL_TABLE_LEN];
    for (id, &(dest_world, dest_pos)) in rows.iter().enumerate() {
        let (screen, col, row_nib) = pipe_helpers::grid_pos_to_dest_nibbles(dest_pos.0, dest_pos.1);
        // W5 and W8 snap per screen and must never carry the centre flag.
        let discrete = dest_world == 4 || dest_world == 7;
        let scrl = pipe_helpers::scroll_nibble(screen, col, discrete);

        // The engine's own encodings, matching what `ObjNorm_PipewayCtlr`
        // would have stored: row and column in the upper nibble, screen bare,
        // and the scroll nibble split the way its ASL/ROL run splits it —
        // bit 3 becomes `Map_Prev_XOff`, bits 2-0 become `Map_Prev_XHi`.
        tables[id] = dest_world;
        tables[PORTAL_MAX + id] = row_nib << 4;
        tables[2 * PORTAL_MAX + id] = screen;
        tables[3 * PORTAL_MAX + id] = col << 4;
        tables[4 * PORTAL_MAX + id] = (scrl & 0x8) << 4;
        tables[5 * PORTAL_MAX + id] = scrl & 0x7;
    }
    rom.write_range(FS_PORTAL_ARRIVAL + PORTAL_TABLE_OFF, &tables);
}

/// Replace `$84A0`'s `JSR Map_Init` with the call to [`STASH_ARRIVAL`].
///
/// **Every arrival needs this, whichever mechanism raised the flag.** It used
/// to live inside the pipe portal's installer, which meant a ROM with telepads
/// and no portals never installed it: the pad raised `ARRIVAL_FLAG`, nothing
/// resolved the id, `World_Num` never changed — so the "teleport" landed you in
/// the world you left — and [`RESTORE_ARRIVAL`] then wrote the untouched,
/// reset-cleared arrival bytes into all ten position variables, parking the
/// player off the map with nowhere to walk. Found on hardware.
fn install_map_init_trampoline(rom: &mut Rom) {
    let mut trampoline = [0u8; MAP_INIT_CALL_LEN];
    trampoline[0] = 0x20; // JSR
    trampoline[1] = STASH_ARRIVAL_CPU as u8;
    trampoline[2] = (STASH_ARRIVAL_CPU >> 8) as u8;
    rom.write_range(MAP_INIT_CALL_OFFSET, &trampoline);
}

// --- Telepads ---------------------------------------------------------------

/// `Map_Operation` — the map's state machine. `$10` starts the enter-level
/// effect.
const MAP_OPERATION: u16 = 0x0729;

/// `PRG010_CEA7` — "begin enter level effect" (CPU `$CEA7` = file 0x14EB7).
/// Its five bytes are two whole instructions:
///
/// ```text
/// A9 10       LDA #$10
/// 8D 29 07    STA Map_Operation
/// ```
///
/// **The one place that knows you have committed to entering a tile**, reached
/// from three branches: the two-player-versus fall-through, the special-tile
/// list, and the ordinary `Tile_AttrTable+4` threshold. All three have just
/// tested `World_Map_Tile`, so it is live.
///
/// **This is in PRG010, and that is the point.** The pipe portal's trigger had
/// to live in PRG030 — the always-mapped bank with eighteen free bytes — because
/// at level exit the banks belong to the level. A hook on the map has PRG010 at
/// `$C000` and PRG011 at `$A000` by construction, so a telepad pays no
/// always-mapped rent at all.
const PAD_HOOK_OFFSET: usize = 0x14EB7;
const PAD_HOOK_LEN: usize = 5;

/// Vanilla bytes at [`PAD_HOOK_OFFSET`].
#[cfg(test)]
#[rustfmt::skip]
const PAD_HOOK_VANILLA: [u8; PAD_HOOK_LEN] = [
    0xA9, 0x10,             // LDA #$10
    0x8D, 0x29, 0x07,       // STA Map_Operation
];

/// Offset of the pad key tables inside [`PAD_ENTER`].
pub(crate) const PAD_TABLE_OFF: usize = 63;

/// Three parallel key tables, sixteen rows — one per shared arrival id. A row
/// says "a pad standing here uses this id", so the id *is* the row index and no
/// pairing table is needed: arrival row `i` holds where pad row `i` sends you.
///
/// `PAD_WORLD` doubles as the row's presence flag: `$FF` is no world, so an
/// unclaimed row can never match and costs no test of its own.
const PAD_WORLD_CPU: u16 = PAD_ENTER_CPU + PAD_TABLE_OFF as u16;
const PAD_Y_CPU: u16 = PAD_WORLD_CPU + PORTAL_MAX as u16;
const PAD_X_CPU: u16 = PAD_Y_CPU + PORTAL_MAX as u16;

/// Teleport instead of entering the tile, when the tile is a pad the table
/// knows about.
///
/// **It writes the portal id into `Map_Entered_XHi` and jumps to `$84A0`**,
/// which is exactly what a pipe portal leaves behind for [`STASH_ARRIVAL`] to
/// find — so the whole arrival path, the completion pack and unpack, and
/// [`RESTORE_ARRIVAL`] are reused with no change at all. A pad is a different
/// way to *reach* the transition, not a different transition.
///
/// No transit room, so no destination-table slot: the twenty-four rooms stay
/// entirely with vanilla's intra-world pipes.
///
/// The key is `(World_Num, row, screen and column)` rather than the world
/// alone, so a world can hold as many pads as there are free arrival ids. The
/// column and screen share one byte — `World_Map_X & $F0` never collides with
/// `World_Map_XHi`, which is a screen index of 0..3.
#[rustfmt::skip]
const PAD_ENTER: [u8; PAD_TABLE_OFF + 3 * PORTAL_MAX] = [
    0xA5, WORLD_MAP_TILE,                                   //  0: LDA World_Map_Tile
    0xC9, TILE_TELEPAD,                                     //  2: CMP #pad tile
    0xD0, 0x24,                                             //  4: BNE +36 -> ordinary
    0xAE, PLAYER_CURRENT as u8,
          (PLAYER_CURRENT >> 8) as u8,                      //  6: LDX Player_Current
    0xA0, (PORTAL_MAX - 1) as u8,                           //  9: LDY #15

    // ----- scan the key tables (11) -----
    0xAD, WORLD_NUM as u8, (WORLD_NUM >> 8) as u8,          // 11: LDA World_Num
    0xD9, PAD_WORLD_CPU as u8, (PAD_WORLD_CPU >> 8) as u8,  // 14: CMP PAD_WORLD,Y
    0xD0, 0x14,                                             // 17: BNE +20 -> next
    0xB5, WORLD_MAP_Y,                                      // 19: LDA World_Map_Y,X
    0x29, 0xF0,                                             // 21: AND #$F0     ; drop sub-tile
    0xD9, PAD_Y_CPU as u8, (PAD_Y_CPU >> 8) as u8,          // 23: CMP PAD_Y,Y
    0xD0, 0x0B,                                             // 26: BNE +11 -> next
    0xB5, WORLD_MAP_X,                                      // 28: LDA World_Map_X,X
    0x29, 0xF0,                                             // 30: AND #$F0     ; column
    0x15, WORLD_MAP_XHI,                                    // 32: ORA World_Map_XHi,X  ; screen
    0xD9, PAD_X_CPU as u8, (PAD_X_CPU >> 8) as u8,          // 34: CMP PAD_X,Y
    0xF0, 0x09,                                             // 37: BEQ +9 -> found

    // ----- next (39) -----
    0x88,                                                   // 39: DEY
    0x10, 0xE1,                                             // 40: BPL -31 -> scan

    // ----- ordinary: the displaced instructions (42) -----
    0xA9, 0x10,                                             // 42: LDA #$10
    0x8D, MAP_OPERATION as u8, (MAP_OPERATION >> 8) as u8,  // 44: STA Map_Operation
    0x60,                                                   // 47: RTS

    // ----- found (48): Y is the arrival id -----
    //
    // No transition flag to raise: `STASH_ARRIVAL` sets `World_Num` from the
    // arrival row at `$84AD`, which is before the pack hook at `$84CD`, so the
    // `World_Num != LIVE_WORLD` compare already sees the jump.
    0x98,                                                   // 48: TYA
    0x9D, MAP_ENTERED_XHI as u8,
          (MAP_ENTERED_XHI >> 8) as u8,                     // 49: STA Map_Entered_XHi,X  ; the id
    0xA9, 0x01,                                             // 52: LDA #$01
    0x8D, ARRIVAL_FLAG as u8, (ARRIVAL_FLAG >> 8) as u8,    // 54: STA ARRIVAL_FLAG
    0xA2, 0xFF,                                             // 57: LDX #$FF
    0x9A,                                                   // 59: TXS
    0x4C, WORLD_MAP_INIT_CPU as u8,
          (WORLD_MAP_INIT_CPU >> 8) as u8,                  // 60: JMP $84A0 (never returns)

    // ----- PAD_WORLD (63), PAD_Y (79), PAD_X (95); $FF world = unclaimed -----
    0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF,
    0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
];

/// A pad: which world it stands in, and where it puts you.
///
/// The tile itself is [`TILE_TELEPAD`] — a byte of the pad's own. The POC used
/// the spade panel, deliberately, as a canary: it is in the engine's own
/// `Map_Completable_Tiles`, so had anything ever marked a pad cell complete the
/// tile would have turned into an M/L panel and the pad would have stopped
/// working. It never did — diverting at *enter* time means `MO_DoLevelClear`
/// never runs — and the canary was then costing the mode its legibility, since
/// a map carries 19 real N-Spade card games and a player cannot tell them from
/// pads. See [`TILE_TELEPAD`] for the byte, the shape, and what it is absent
/// from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Telepad {
    /// The world the pad stands in, 0-based.
    pub world: u8,
    /// The destination world, 0-based.
    pub dest_world: u8,
    /// Where to arrive on that world's map, as `(grid_row, grid_col)`.
    pub dest_pos: (usize, usize),
    /// Where the pad stands, as `(grid_row, grid_col)`. **Load-bearing**: it is
    /// the key the routine matches the player's live position against, which is
    /// what lets one world hold several pads.
    pub src_pos: (usize, usize),
}

/// Install the telepads: the enter hook, its key tables, and the hook site.
///
/// A pad's index in `telepads` **is** its arrival id, so pad `n`'s key row and
/// arrival row are both row `n` — see [`PAD_ENTER`]'s tables.
fn apply_telepads(rom: &mut Rom, telepads: &[Telepad]) {
    let mut code = PAD_ENTER;
    for (id, pad) in telepads.iter().enumerate() {
        // The same encoder the arrival rows use, because `World_Map_Y & $F0` is
        // `Map_Entered_Y` and `World_Map_X & $F0` is `Map_Entered_X`. One
        // source of truth for the map's coordinate encoding, and it is the one
        // the portals already proved on hardware.
        let (screen, col, row_nib) =
            pipe_helpers::grid_pos_to_dest_nibbles(pad.src_pos.0, pad.src_pos.1);
        code[PAD_TABLE_OFF + id] = pad.world;
        code[PAD_TABLE_OFF + PORTAL_MAX + id] = row_nib << 4;
        code[PAD_TABLE_OFF + 2 * PORTAL_MAX + id] = (col << 4) | screen;
    }
    rom.write_range(FS_PAD_ENTER, &code);

    let mut hook = [0xEA_u8; PAD_HOOK_LEN];
    hook[0] = 0x20; // JSR
    hook[1] = PAD_ENTER_CPU as u8;
    hook[2] = (PAD_ENTER_CPU >> 8) as u8;
    rom.write_range(PAD_HOOK_OFFSET, &hook);
}

/// Decode the pad key tables back out of a finished ROM: one
/// `(world, grid row, grid column)` per claimed row, in arrival-id order.
///
/// The layout knowledge lives here rather than in the tests that read it, so a
/// change to the key encoding cannot leave one quietly decoding the old shape —
/// the same argument as [`super::foreign_locks::decode_rows`], and the same
/// reason: three separate copies of this decode had already grown.
///
/// [`apply_telepads`] deals ids from 0 upward, so a claimed row can never
/// follow an unclaimed one and the vector index is the arrival id. `$FF` in
/// `PAD_WORLD` is the row's "unclaimed" flag — there is no world 255 — so an
/// un-patched ROM full of `$FF` filler decodes to nothing.
#[cfg(test)]
pub(crate) fn decode_pad_rows(rom: &Rom) -> Vec<(usize, usize, usize)> {
    let table = FS_PAD_ENTER + PAD_TABLE_OFF;
    (0..PORTAL_MAX)
        .filter(|id| rom.read_byte(table + id) != 0xFF)
        .map(|id| {
            let world = rom.read_byte(table + id) as usize;
            // Y is `(grid_row + 2) << 4`; the X byte packs the column within
            // its screen in the high nibble and the screen index in the low.
            let y = rom.read_byte(table + PORTAL_MAX + id);
            let x = rom.read_byte(table + 2 * PORTAL_MAX + id);
            let row = (y >> 4) as usize - 2;
            let col = (x & 0x0F) as usize * 16 + (x >> 4) as usize;
            (world, row, col)
        })
        .collect()
}

#[cfg(test)]
mod asm_checks {
    use mos6502::instruction::Ricoh2a03;
    use mos6502::memory::{Bus, Memory};

    use super::*;
    use crate::randomize::rom_data::asm;

    /// `completion_bits` owns the wipe site; this module only writes the
    /// arrival restore into the padding it leaves, which is a shared 10-byte
    /// run and exactly the kind of arrangement that breaks quietly when either
    /// side moves. Nothing is hooked or displaced here — the assertion is that
    /// the other module left the three NOPs the restore's call goes into.
    #[test]
    fn completion_bits_leaves_room_for_the_arrival_restore() {
        let Some(rom) = load_vanilla() else { return };

        let mut patched = rom.clone();
        completion_bits::apply(&mut patched);
        let wipe = patched.read_range(WIPE_OFFSET, WIPE_LEN).to_vec();

        assert_eq!(
            &wipe[3..6],
            &[0xEA, 0xEA, 0xEA],
            "completion_bits must leave NOPs at the wipe site for the arrival restore"
        );
    }

    #[test]
    fn restore_arrival_is_well_formed() {
        asm::check(&RESTORE_ARRIVAL)
            .allocation(FS_RESTORE_ARRIVAL)
            .origin(RESTORE_ARRIVAL_CPU)
            .zero_page(NMI_SAFE_MAX, &[])
            .assert_ok();
    }

    /// The arrival restore must actually be reached, or a portal's arrival is
    /// silently dropped and you land on the destination world's start tile —
    /// the version of this that is worse than no transit room at all.
    ///
    /// It rides in the padding `completion_bits` leaves at the wipe site, which
    /// is a shared 10-byte run and exactly the kind of arrangement that breaks
    /// quietly when either side moves.
    #[test]
    fn the_wipe_site_calls_the_arrival_restore() {
        let Some(rom) = load_vanilla() else { return };
        let mut patched = rom.clone();
        apply(
            &mut patched,
            &[Telepad { world: 1, dest_world: 6, dest_pos: (5, 12), src_pos: (0, 4) }],
        );
        let wipe = patched.read_range(WIPE_OFFSET, WIPE_LEN);
        assert_eq!(wipe[3], 0x20, "the arrival restore must be called, not fallen into");
        assert_eq!(
            u16::from_le_bytes([wipe[4], wipe[5]]),
            RESTORE_ARRIVAL_CPU,
            "and the call must name the restore"
        );
    }

    /// Every variable `Map_Init` sets from `Map_Y_Starts` has to be rewritten.
    /// Missing one leaves the player half-moved — drawn at the portal's tile
    /// but resuming at the world's start after a death, or vice versa.
    #[test]
    fn restore_covers_every_position_variable_map_init_writes() {
        let written: Vec<u16> = RESTORE_ARRIVAL
            .windows(3)
            .filter(|w| w[0] == 0x8D)
            .map(|w| u16::from_le_bytes([w[1], w[2]]))
            .collect();
        for (addr, name) in [
            (MAP_ENTERED_Y, "Map_Entered_Y"),
            (MAP_ENTERED_X, "Map_Entered_X"),
            (MAP_ENTERED_XHI, "Map_Entered_XHi"),
            (MAP_PREVIOUS_Y, "Map_Previous_Y"),
            (MAP_PREVIOUS_X, "Map_Previous_X"),
            (MAP_PREVIOUS_XHI, "Map_Previous_XHi"),
            (MAP_PREV_XOFF, "Map_Prev_XOff"),
            (MAP_PREV_XHI, "Map_Prev_XHi"),
            (MAP_PREV_XOFF2, "Map_Prev_XOff2"),
            (MAP_PREV_XHI2, "Map_Prev_XHi2"),
        ] {
            assert!(written.contains(&addr), "restore never writes {name} (${addr:04X})");
        }
    }

    /// The stash and its six tables share one allocation, so the check has to
    /// see the whole span: `data_from` is what stops the tables being decoded
    /// as code, and `allocation` is what catches the pair outgrowing PRG011's
    /// row together.
    #[test]
    fn stash_arrival_is_well_formed() {
        let mut span = [0u8; PORTAL_TABLE_OFF + PORTAL_TABLE_LEN];
        span[..PORTAL_TABLE_OFF].copy_from_slice(&STASH_ARRIVAL);
        asm::check(&span)
            .allocation(FS_PORTAL_ARRIVAL)
            .origin(STASH_ARRIVAL_CPU)
            .data_from(PORTAL_TABLE_OFF)
            .zero_page(NMI_SAFE_MAX, &[])
            .assert_ok();
    }

    /// Every table read in the stash names an address inside the tables that
    /// follow it, and they are read in the order the tables are laid out.
    ///
    /// Written out because the addresses are computed from
    /// [`PORTAL_TABLE_OFF`]: get that constant wrong and the routine indexes
    /// its own instructions, which decodes and assembles perfectly well.
    ///
    /// The reads are named by offset rather than found by scanning for `$BD`,
    /// because `$BD` is also the high byte of every one of these addresses —
    /// the routine lives at `$BDCB`. A byte scan finds eight "instructions"
    /// where there are seven.
    #[test]
    fn stash_reads_its_own_tables_in_order() {
        let operand = |at: usize| {
            assert_eq!(STASH_ARRIVAL[at], 0xBD, "offset {at} is not an LDA abs,X");
            u16::from_le_bytes([STASH_ARRIVAL[at + 1], STASH_ARRIVAL[at + 2]])
        };
        assert_eq!(operand(8), MAP_ENTERED_XHI, "the id comes from Map_Entered_XHi");

        let first = STASH_ARRIVAL_CPU + PORTAL_TABLE_OFF as u16;
        let last = first + (PORTAL_TABLE_LEN - 1) as u16;
        for (at, want, name) in [
            (12, PORTAL_WORLD_CPU, "PORTAL_WORLD"),
            (18, PORTAL_Y_CPU, "PORTAL_Y"),
            (24, PORTAL_XHI_CPU, "PORTAL_XHI"),
            (30, PORTAL_X_CPU, "PORTAL_X"),
            (36, PORTAL_SCRL_CPU, "PORTAL_SCRL"),
            (42, PORTAL_SCRH_CPU, "PORTAL_SCRH"),
        ] {
            let got = operand(at);
            assert_eq!(got, want, "offset {at} should read {name}");
            assert!(
                (first..=last).contains(&got),
                "{name} at ${got:04X} is outside the tables (${first:04X}-${last:04X})"
            );
        }
    }

    #[test]
    fn pad_enter_is_well_formed() {
        asm::check(&PAD_ENTER)
            .allocation(FS_PAD_ENTER)
            .origin(PAD_ENTER_CPU)
            .data_from(PAD_TABLE_OFF)
            // The tile byte and the three live position bytes are engine
            // variables the scan reads; it writes nothing to zero page at all.
            // Named rather than bounded, or a bound of `$E5` would permit the
            // whole page. See `asm::Routine::zero_page`.
            .zero_page(NMI_SAFE_MAX, &[WORLD_MAP_TILE, WORLD_MAP_Y, WORLD_MAP_XHI, WORLD_MAP_X])
            .assert_ok();
    }

    /// The pad hook displaces two whole instructions and replays them, so a
    /// tile that is not a pad enters its level exactly as before.
    #[test]
    fn pad_hook_displaces_whole_instructions() {
        let Some(rom) = load_vanilla() else { return };
        assert_eq!(
            rom.read_range(PAD_HOOK_OFFSET, PAD_HOOK_LEN),
            PAD_HOOK_VANILLA,
            "PRG010_CEA7 has moved"
        );
        assert_eq!(
            PAD_ENTER[42..47],
            PAD_HOOK_VANILLA,
            "the ordinary path must replay what the hook overwrote"
        );

        let mut jsr = [0xEA_u8; PAD_HOOK_LEN];
        jsr[0] = 0x20;
        jsr[1] = PAD_ENTER_CPU as u8;
        jsr[2] = (PAD_ENTER_CPU >> 8) as u8;
        asm::check(&PAD_ENTER)
            .allocation(FS_PAD_ENTER)
            .origin(PAD_ENTER_CPU)
            .data_from(PAD_TABLE_OFF)
            .zero_page(NMI_SAFE_MAX, &[WORLD_MAP_TILE, WORLD_MAP_Y, WORLD_MAP_XHI, WORLD_MAP_X])
            .hook(&PAD_HOOK_VANILLA, 0, &jsr)
            .assert_ok();
    }

    /// The three tables the scan indexes are its own, and they are laid out
    /// where the constants say. The addresses are computed from
    /// [`PAD_TABLE_OFF`]: get it wrong and the routine scans its own
    /// instructions, which decodes and runs perfectly well.
    #[test]
    fn the_pad_scan_reads_its_own_key_tables() {
        let operand = |at: usize| {
            assert_eq!(PAD_ENTER[at], 0xD9, "offset {at} is not a CMP abs,Y");
            u16::from_le_bytes([PAD_ENTER[at + 1], PAD_ENTER[at + 2]])
        };
        let first = PAD_ENTER_CPU + PAD_TABLE_OFF as u16;
        let last = first + (3 * PORTAL_MAX - 1) as u16;
        for (at, want, name) in
            [(14, PAD_WORLD_CPU, "PAD_WORLD"), (23, PAD_Y_CPU, "PAD_Y"), (34, PAD_X_CPU, "PAD_X")]
        {
            let got = operand(at);
            assert_eq!(got, want, "offset {at} should read {name}");
            assert!(
                (first..=last).contains(&got),
                "{name} at ${got:04X} is outside the tables (${first:04X}-${last:04X})"
            );
        }
        assert_eq!(PAD_ENTER.len(), PAD_TABLE_OFF + 3 * PORTAL_MAX);
        assert!(
            PAD_ENTER[PAD_TABLE_OFF..PAD_TABLE_OFF + PORTAL_MAX].iter().all(|&b| b == 0xFF),
            "every row starts unclaimed, and $FF is a world nothing can be in"
        );
    }

    /// **Nothing the world maze writes touches the ROM's pipe data.** That was
    /// the whole reason the pipe portal was retired in favour of pads, so it is
    /// asserted rather than assumed: all four pipe destination tables and every
    /// world pointer table must come out byte-identical to vanilla.
    #[test]
    fn the_maze_leaves_every_pipe_alone() {
        let Some(rom) = load_vanilla() else { return };
        let pads = [
            Telepad { world: 0, dest_world: 4, dest_pos: (6, 6), src_pos: (4, 8) },
            Telepad { world: 4, dest_world: 0, dest_pos: (4, 8), src_pos: (6, 6) },
        ];
        let mut patched = rom.clone();
        apply(&mut patched, &pads);

        use crate::randomize::rom_data::{PIPE_MAP_SCRL_XHI, PIPE_MAP_X, PIPE_MAP_XHI, PIPE_MAP_Y};
        for (base, name) in [
            (PIPE_MAP_XHI, "PIPE_MAP_XHI"),
            (PIPE_MAP_Y, "PIPE_MAP_Y"),
            (PIPE_MAP_X, "PIPE_MAP_X"),
            (PIPE_MAP_SCRL_XHI, "PIPE_MAP_SCRL_XHI"),
        ] {
            assert_eq!(
                patched.read_range(base, 24),
                rom.read_range(base, 24),
                "{name} changed — a telepad must spend no transit room"
            );
        }
        for world in &crate::randomize::rom_data::WORLDS {
            let n = world.entry_count * 5;
            assert_eq!(
                patched.read_range(world.rowtype_offset, n),
                rom.read_range(world.rowtype_offset, n),
                "a world pointer table changed — a telepad repoints nothing"
            );
        }

        // The key tables describe where each pad stands, row index = arrival id.
        let table = FS_PAD_ENTER + PAD_TABLE_OFF;
        assert_eq!(patched.read_byte(table), 0, "row 0 is W1's pad");
        assert_eq!(patched.read_byte(table + PORTAL_MAX), 0x60, "W1 pad row 4 -> (4+2)<<4");
        assert_eq!(patched.read_byte(table + 2 * PORTAL_MAX), 0x80, "W1 pad col 8, screen 0");
        assert_eq!(patched.read_byte(table + 1), 4, "row 1 is W5's pad");
        assert_eq!(patched.read_byte(table + PORTAL_MAX + 1), 0x80, "W5 pad row 6");
        assert_eq!(patched.read_byte(table + 2 * PORTAL_MAX + 1), 0x60, "W5 pad col 6, screen 0");
        for row in 2..PORTAL_MAX {
            assert_eq!(patched.read_byte(table + row), 0xFF, "row {row} is unclaimed");
        }
    }

    /// **A pad needs the `Map_Init` trampoline**, and without pads nothing
    /// should install one.
    ///
    /// The trampoline is what resolves the arrival id and sets `World_Num`. It
    /// used to be written inside the pipe portal's installer, so a ROM with
    /// telepads and no portals never got it: the pad raised `ARRIVAL_FLAG`,
    /// nothing resolved the id, `World_Num` never changed — the teleport landed
    /// you in the world you left — and `RESTORE_ARRIVAL` then wrote the
    /// reset-cleared arrival bytes into all ten position variables, parking the
    /// player off the map. Found on hardware.
    #[test]
    fn a_telepad_installs_the_map_init_trampoline() {
        let Some(rom) = load_vanilla() else { return };
        let pad = Telepad { world: 1, dest_world: 6, dest_pos: (5, 12), src_pos: (0, 4) };

        let mut patched = rom.clone();
        apply(&mut patched, &[pad]);
        assert_eq!(
            patched.read_range(MAP_INIT_CALL_OFFSET, MAP_INIT_CALL_LEN),
            [0x20, STASH_ARRIVAL_CPU as u8, (STASH_ARRIVAL_CPU >> 8) as u8],
            "$84AD must call STASH_ARRIVAL, or nothing resolves the arrival id"
        );

        let mut patched = rom.clone();
        apply(&mut patched, &[]);
        assert_eq!(
            patched.read_range(MAP_INIT_CALL_OFFSET, MAP_INIT_CALL_LEN),
            [0x20, MAP_INIT_CPU as u8, (MAP_INIT_CPU >> 8) as u8],
            "no pads: Map_Init must still be called directly"
        );
    }

    /// Without pads, the map's enter-level path is untouched.
    #[test]
    fn telepads_are_opt_in() {
        let Some(rom) = load_vanilla() else { return };
        let mut patched = rom.clone();
        apply(&mut patched, &[]);
        assert_eq!(
            patched.read_range(PAD_HOOK_OFFSET, PAD_HOOK_LEN),
            PAD_HOOK_VANILLA,
            "the enter-level hook must not be installed unless asked for"
        );
    }

    // --- Executing the pad routine -------------------------------------
    //
    // `asm::check` proves the bytes decode; it cannot see that they compute the
    // right thing, and four planted mutations inside the array survived every
    // structural test. This routine is the rare one that can be *run*: it calls
    // nothing, touching only `World_Map_Tile`, `World_Num`, its own table,
    // `Player_Current` and three RAM bytes. So run it.

    /// Where `call_pad_enter` parks its return address; reaching it means the
    /// routine took the ordinary path and returned.
    const PAD_SENTINEL: u16 = 0x0F00;

    /// Run `PAD_ENTER` and say whether it teleported (reached `$84A0`) or
    /// returned to the caller.
    fn call_pad_enter(cpu: &mut mos6502::cpu::CPU<Memory, Ricoh2a03>) -> bool {
        let ret = PAD_SENTINEL.wrapping_sub(1);
        cpu.memory.set_byte(0x01FF, (ret >> 8) as u8);
        cpu.memory.set_byte(0x01FE, ret as u8);
        cpu.registers.stack_pointer = mos6502::registers::StackPointer(0xFD);
        cpu.registers.program_counter = PAD_ENTER_CPU;
        for _ in 0..10_000 {
            match cpu.registers.program_counter {
                WORLD_MAP_INIT_CPU => return true,
                PAD_SENTINEL => return false,
                _ => {
                    cpu.single_step();
                }
            }
        }
        panic!("PAD_ENTER ran away");
    }

    /// **The row key, checked against the engine's own bytes rather than
    /// against my arithmetic.**
    ///
    /// `GameOver_AlignToStartY` stores `Map_Y_Starts[world]` straight into
    /// `World_Map_Y`, and `Map_Init` forces `World_Map_X` to `$20`. So for
    /// every world the key `apply` writes for the START tile's grid position
    /// must come out as exactly those bytes. The grid position is read from the
    /// map layout and the bytes from the engine's own tables, so nothing here
    /// shares a source with the encoder under test — and it goes through
    /// `apply`, because the bug was in how the pad table *used* the encoding.
    ///
    /// This is the test that was missing. The first cut derived the key from
    /// `Map_GetTile`'s `SUB #16` and landed one row short — and the CPU fixture
    /// computed the player's position the same wrong way, so it agreed with the
    /// bug and every pad silently entered its spade game instead. A fixture
    /// that shares the code's assumption cannot test that assumption.
    #[test]
    fn the_pad_row_key_matches_the_engine_own_start_bytes() {
        let Some(rom) = load_vanilla() else { return };
        for world in 0..8 {
            let grid = crate::randomize::rom_data::read_tile_grid(&rom, world);
            let start = (0..crate::randomize::rom_data::ROWS)
                .flat_map(|r| (0..grid.cols).map(move |c| (r, c)))
                .find(|&(r, c)| grid.get(r, c) == crate::randomize::rom_data::TILE_START)
                .unwrap_or_else(|| panic!("W{} has no START tile", world + 1));

            // Through `apply`, not through the encoder: the bug was in how the
            // pad table used the encoding, and a test that recomputes the
            // encoding itself would have agreed with it.
            let pad =
                Telepad { world: world as u8, dest_world: 0, dest_pos: (2, 2), src_pos: start };
            let mut patched = rom.clone();
            apply(&mut patched, &[pad]);
            let key = patched.read_byte(FS_PAD_ENTER + PAD_TABLE_OFF + PORTAL_MAX);
            let engine = rom.read_byte(crate::randomize::rom_data::MAP_Y_STARTS_OFF + world);

            // The column and screen have their own engine oracle: `Map_Init`
            // forces `World_Map_X` to $20 for every world, and
            // `GameOver_ReturnToStartX` calls $20 "the fixed start point X".
            // That is column 2 of screen 0, so the packed key must be $20 too.
            assert_eq!(start.1, 2, "W{}'s START tile is not at column 2", world + 1);
            assert_eq!(
                patched.read_byte(FS_PAD_ENTER + PAD_TABLE_OFF + 2 * PORTAL_MAX),
                0x20,
                "W{}: the engine puts the player at World_Map_X $20 on screen 0, so the packed \
                 column-and-screen key must be $20",
                world + 1
            );
            assert_eq!(
                key,
                engine,
                "W{}: START is at grid row {}, so the pad row key is ${key:02X}, but the engine \
                 puts the player at World_Map_Y ${engine:02X}",
                world + 1,
                start.0
            );
        }
    }

    /// Set up a CPU with the routine, its key tables, and the machine state the
    /// engine would have when a player presses A on a tile.
    ///
    /// `at` is where the player is standing, `pads` the rows to claim as
    /// `(row index / arrival id, world, grid position)`.
    fn pad_cpu(
        tile: u8,
        world: u8,
        player: u8,
        at: (usize, usize),
        sub_tile: u8,
        pads: &[(usize, u8, (usize, usize))],
    ) -> mos6502::cpu::CPU<Memory, Ricoh2a03> {
        let mut code = PAD_ENTER;
        for &(id, w, at) in pads {
            let (screen, col, row_nib) = pipe_helpers::grid_pos_to_dest_nibbles(at.0, at.1);
            code[PAD_TABLE_OFF + id] = w;
            code[PAD_TABLE_OFF + PORTAL_MAX + id] = row_nib << 4;
            code[PAD_TABLE_OFF + 2 * PORTAL_MAX + id] = (col << 4) | screen;
        }
        let mut mem = Memory::new();
        mem.set_bytes(PAD_ENTER_CPU, &code);
        mem.set_byte(WORLD_MAP_TILE as u16, tile);
        mem.set_byte(WORLD_NUM, world);
        mem.set_byte(PLAYER_CURRENT, player);
        // The engine's own encoding, plus a sub-tile offset the key must ignore.
        let (screen, col, row_nib) = pipe_helpers::grid_pos_to_dest_nibbles(at.0, at.1);
        mem.set_byte(WORLD_MAP_Y as u16 + player as u16, (row_nib << 4) | sub_tile);
        mem.set_byte(WORLD_MAP_X as u16 + player as u16, (col << 4) | sub_tile);
        mem.set_byte(WORLD_MAP_XHI as u16 + player as u16, screen);
        // Poison what the routine should write, so "unchanged" is visible.
        mem.set_byte(MAP_ENTERED_XHI, 0xAA);
        mem.set_byte(MAP_ENTERED_XHI + 1, 0xAA);
        mem.set_byte(ARRIVAL_FLAG, 0xAA);
        mem.set_byte(MAP_OPERATION, 0xAA);
        mos6502::cpu::CPU::new(mem, Ricoh2a03)
    }

    /// Standing on a pad teleports: it hands the arrival id to the place
    /// `STASH_ARRIVAL` reads, raises the arrival flag, and never sets
    /// `Map_Operation` — the map must not also start an enter-level effect.
    ///
    /// Run with a sub-tile pixel offset on both axes, because the player's
    /// position is a pixel coordinate and the key has to mask it off.
    #[test]
    fn a_pad_hands_over_its_id_and_teleports() {
        let pads = &[(0usize, 0u8, (4usize, 8usize)), (5, 4, (6, 22)), (15, 8, (0, 47))];
        for &(id, world, at) in pads {
            for sub_tile in [0x00, 0x0F] {
                let mut cpu = pad_cpu(TILE_TELEPAD, world, id as u8 % 2, at, sub_tile, pads);
                let player = id as u8 % 2;
                assert!(
                    call_pad_enter(&mut cpu),
                    "W{} pad at {at:?} did not teleport (sub-tile {sub_tile:#04X})",
                    world + 1
                );
                assert_eq!(
                    cpu.memory.get_byte(MAP_ENTERED_XHI + player as u16),
                    id as u8,
                    "the arrival id must land in the current player's Map_Entered_XHi"
                );
                assert_eq!(
                    cpu.memory.get_byte(MAP_ENTERED_XHI + (1 - player) as u16),
                    0xAA,
                    "and not in the other player's"
                );
                assert_eq!(cpu.memory.get_byte(ARRIVAL_FLAG), 1, "ARRIVAL_FLAG");
                assert_eq!(
                    cpu.memory.get_byte(MAP_OPERATION),
                    0xAA,
                    "Map_Operation must not be touched on the teleport path"
                );
            }
        }
    }

    /// **One world, several pads** — the thing keying on `World_Num` could not
    /// do. Three pads in World 3, each reached from its own tile.
    #[test]
    fn one_world_can_hold_several_pads() {
        let pads = &[
            (0usize, 2u8, (1usize, 3usize)),
            (1, 2, (5, 19)),
            (2, 2, (8, 40)),
            (3, 6, (5, 19)), // same tile, different world: must not be confused
        ];
        for &(id, world, at) in pads {
            let mut cpu = pad_cpu(TILE_TELEPAD, world, 0, at, 0, pads);
            assert!(call_pad_enter(&mut cpu), "pad {id} at {at:?} did not teleport");
            assert_eq!(
                cpu.memory.get_byte(MAP_ENTERED_XHI),
                id as u8,
                "W{} pad at {at:?} must use arrival id {id}",
                world + 1
            );
        }
    }

    /// Any other tile enters its level as vanilla did; so does a pad tile the
    /// table does not know about, whether because the world has no pads at all
    /// or because this is the wrong tile in a world that does. Each is a path a
    /// mistake makes teleport, and each must leave the flags alone.
    #[test]
    fn a_tile_that_is_not_a_known_pad_enters_its_level() {
        let pads = &[(0usize, 2u8, (5usize, 19usize))];
        let cases: [(&str, u8, u8, (usize, usize)); 6] = [
            ("an ordinary level panel", 0x03, 2, (5, 19)),
            ("a fortress", 0x67, 2, (5, 19)),
            ("a pipe", 0xBC, 2, (5, 19)),
            ("a pad tile in a world with no pads", TILE_TELEPAD, 5, (5, 19)),
            ("a pad tile one row off", TILE_TELEPAD, 2, (4, 19)),
            ("a pad tile on the wrong screen", TILE_TELEPAD, 2, (5, 3)),
        ];
        for (what, tile, world, at) in cases {
            let mut cpu = pad_cpu(tile, world, 0, at, 0, pads);
            assert!(!call_pad_enter(&mut cpu), "{what} teleported");
            assert_eq!(cpu.memory.get_byte(MAP_OPERATION), 0x10, "{what}: Map_Operation");
            assert_eq!(cpu.memory.get_byte(ARRIVAL_FLAG), 0xAA, "{what}: ARRIVAL_FLAG touched");
            assert_eq!(
                cpu.memory.get_byte(MAP_ENTERED_XHI),
                0xAA,
                "{what}: Map_Entered_XHi touched"
            );
        }
    }

    fn load_vanilla() -> Option<Rom> {
        let data = std::fs::read("roms/Super Mario Bros. 3 (USA) (Rev 1).nes").ok()?;
        Rom::from_bytes(&data).ok()
    }
}
