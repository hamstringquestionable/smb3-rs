//! Position-keyed lock table: one mechanism for every lock a fortress opens.
//!
//! This replaces vanilla's fortress-FX slot tables and the world maze's separate
//! cross-world lock table with a single table keyed by *where the player is
//! standing*. `docs/fx_table_redesign.md` is the design note; this module is its
//! stage 1.
//!
//! # Why the slots had to go
//!
//! Vanilla identifies a lock by a **global slot index**, smuggled out of the
//! fortress as the high nibble of Boom-Boom's spawn Y byte, turned into a
//! per-world ordinal by the `(?)` orb, and resolved through
//! `FortressFXBase_ByWorld` + `FortressFX_W1..W8` into an index that addresses
//! seven parallel tables. The census saturates every cap that chain imposes: 17
//! fortresses against 17 slots, and World 8's four locks against a four-wide
//! per-world row. There is no headroom, `0x00` means both "slot 0" and "unused",
//! and a lock could be keyed to nothing but a fortress.
//!
//! Worse, the maze's cross-world locks were a *second* mechanism keyed
//! differently, and keeping the two in step had already produced a bug that
//! survived the whole life of the mode.
//!
//! # The trigger is vanilla's own, and that is the point
//!
//! **The `(?)` orb still arms the effect.** `ObjInit_BoomBoom` copies the enemy
//! record's Y-hi nibble into `Objects_Var4` and the orb stores it into
//! `Map_DoFortressFX`; map operation 8 runs when it is non-zero. Nothing about
//! that changes, and all 17 of vanilla's Boom-Booms already carry a non-zero
//! nibble, so **this module writes no enemy data at all.**
//!
//! Keeping vanilla's trigger is what makes the mechanism universal. Both halves
//! of `MO_DoLevelClear` — the map-cell clear *and* the map-object poof — fall
//! through to one shared exit:
//!
//! ```text
//! PRG011_ABBE:
//!     LDA <Map_ClearLevelFXCnt
//!     BNE PRG011_ABCC
//!     LDA #$08
//!     STA Map_Operation          ; op 8 = MO_DoFortressFX, either way
//! ```
//!
//! So a World 8 tank and a stone fortress reach the effect identically. An
//! earlier cut of this rework instead keyed on `Map_MarkLevelComplete`'s
//! fortress branch, which is gated on the tile under the player being rubble —
//! and **17.6% of locks in every seed are keyed to a W8 army sprite**, whose
//! cell the writer deliberately blanks to a path node. Those locks were dead.
//! Reading the position at op 8 has no such gate: the player is standing on the
//! thing they just cleared, whatever tile is under it.
//!
//! # The key, and the table
//!
//! ```text
//! key0  (World_Map_Y & $F0) | World_Num      (grid row + 2) << 4, world
//! key1  (World_Map_X & $F0) | World_Map_XHi  col in screen << 4, screen
//! t0    the target, two bytes; see `home_target` / `away_target`
//! t1
//! ```
//!
//! Both key bytes are the player's live position with its sub-tile offsets
//! masked off — the same `Map_Entered_Y` encoding the pipe tables and arrival
//! rows use, and the same packed column byte vanilla's own FX table stored. No
//! fort-id numbering has to be invented on either side, and unlike
//! `Map_MarkLevelComplete`'s completion row (seven entries, grid rows 7 and 8
//! folded onto one index) this distinguishes every row.
//!
//! A hit does one of two things:
//!
//! * **home** — the target is in the world the player is standing in. Resolve
//!   the cell and animate it.
//! * **away** — the target is in another world. Set the bit in
//!   [`super::completion_bits`]' packed store and stop; nobody is there to see
//!   it, and the flash and poof would be drawn at a position that entry does not
//!   carry.
//!
//! `Away` entries are emitted first, so the console-side test is `CPX #boundary`
//! against the scan's own descending index.
//!
//! # Nothing about the target is stored that the map already knows
//!
//! The old slot tables cached 153 bytes of arithmetic over the target cell: the
//! VRAM address (`$2880 + row*64 + col_in_screen*2`), the `Map_Completions`
//! column and bit, the CHR quadrants, and the replacement tile. All of it is
//! derived now, which also removes a bug class — the immediate change and the
//! after-reload change were independent sources of truth that happened to agree.
//!
//! # But PRG012 is not banked in — hence the mirror
//!
//! `Map_Removable_Tiles`/`Map_RemoveTo_Tiles` and the metatile quadrant tables
//! all live in PRG012, and during map play `$A000` is PRG011 and `$C000` is
//! PRG010. Neither lookup is reachable from the effect. [`mirror_bytes`] copies
//! the 48 bytes that matter into the PRG011 run this rework freed, so one index
//! drives the tile swap *and* the CHR patterns.

use std::collections::HashMap;

use crate::rom::Rom;

use super::completion_bits::{CompletionMap, HALF_LEN, PLANE_RESERVE};
#[cfg(test)]
use super::rom_data::NMI_SAFE_MAX;
use super::rom_data::{
    self, FS_COMPLETION_BASES, FS_FORTRESS_FX, FS_LOCK_ENTRIES, FS_LOCK_MIRROR, FS_MAP_REMOVABLE,
    FS_ML_RANGE, MAP_COMPLETE_BIT_CPU, MAP_COMPLETE_BITS, MAP_COMPLETIONS, PLAYER_CURRENT,
    PRG012_FILE_BASE, REMOVABLE_STRIDE, WORLD_NUM, prg_bank_file_to_cpu, prg010_file_to_cpu,
    prg011_file_to_cpu,
};

// --- Siting -------------------------------------------------------------

/// The mirror sits in PRG011, in the run the retired `Map_MarkLevelComplete`
/// hook gave back. Both map banks are mapped for the whole map (`$84A0` sets
/// PAGE_A000 = 11 and PAGE_C000 = 10), so PRG010 code reads it freely — and
/// keeping it out of PRG010 leaves the whole freed fortress-FX block to the
/// routine, which is the thing that grows.
const MIRROR_CPU: u16 = prg011_file_to_cpu(FS_LOCK_MIRROR);
/// `Map_Removable_Tiles`, mirrored: every obstacle tile.
const MIRROR_REMOVABLE: u16 = MIRROR_CPU;
/// `Map_RemoveTo_Tiles`, mirrored: what each obstacle becomes.
const MIRROR_REMOVE_TO: u16 = MIRROR_CPU + REMOVABLE_COUNT as u16;
/// Four CHR quadrants per remove-to tile, in the order the effect queues them.
const MIRROR_PATTERNS: u16 = MIRROR_CPU + 2 * REMOVABLE_COUNT as u16;
/// Bytes the mirror occupies: six per entry, and it is packed rather than
/// strided because the whole thing is rewritten from Rust every run.
const MIRROR_LEN: usize = REMOVABLE_COUNT * 6;

/// The replacement `MO_DoFortressFX`, at the head of the freed block.
const FORTRESS_FX_CPU: u16 = prg010_file_to_cpu(FS_FORTRESS_FX);

/// The entry table, in the run the old screen-check patch vacated.
const ENTRIES_CPU: u16 = prg010_file_to_cpu(FS_LOCK_ENTRIES);

/// `.word MO_DoFortressFX` — entry 8 of the map-operation jump table at
/// `prg010.asm:913`, the only thing in the ROM that names the routine. That is
/// what makes a wholesale rewrite possible: repoint this word and the vanilla
/// routine and its 236 bytes of tables become free space.
const MAP_OP8_VECTOR: usize = 0x144D2;

/// What stands there in vanilla: `$C8A9`, the head of `MO_DoFortressFX`.
#[cfg(test)]
const MAP_OP8_VANILLA: u16 = 0xC8A9;

/// Offset of the `LDX #` immediate that sizes the scan: `(entries - 1) * 4`.
const COUNT_OPERAND: usize = 36;

/// Offset of the `CPX #` immediate that splits away entries from home ones.
const BOUNDARY_OPERAND: usize = 71;

/// Bytes reserved for the entry table. Four per entry.
const ENTRIES_RESERVED: usize = 112;

/// How many locks a build can key. Today's ceiling is the 17-card fortress deck
/// — and every fortress keys exactly one lock, so 17 is also the floor. The
/// reservation is deliberately larger so stage 2's fort deja vu does not have to
/// relocate anything.
pub const MAX_ENTRIES: usize = ENTRIES_RESERVED / ENTRY_LEN;

/// `[key0, key1, t0, t1]`.
const ENTRY_LEN: usize = 4;

// --- Engine symbols the routine names -----------------------------------

/// Base of [`super::completion_bits`]' packed store.
///
/// Private over there, so it is restated here and pinned by
/// `the_packed_base_matches_completion_bits`, which reads the address back out
/// of the `PACK_PLANE` bytes that module writes to the ROM.
const PACKED: u16 = 0x7997;

/// The mirror plane: one [`PLANE_RESERVE`] on from Mario's.
const PACKED_MIRROR: u16 = PACKED + PLANE_RESERVE as u16;

/// `Map_Removable_Tiles` / `Map_RemoveTo_Tiles` **as this randomizer sites
/// them** — the engine's own answer to "what does this obstacle become", read by
/// `Map_Reload_with_Completions` on every map load.
///
/// Vanilla puts the two 8-entry tables at `$A437` and `$A43F` with
/// `Map_Completable_Tiles` immediately after at `$A447`, so neither can be
/// extended a byte. [`relocate_removable_tables`] moves them into
/// [`FS_MAP_REMOVABLE`] with a fixed stride and repoints the two instructions
/// that read them; from there an obstacle variant costs one row.
const MAP_REMOVABLE_TILES: usize = FS_MAP_REMOVABLE;
const MAP_REMOVE_TO_TILES: usize = FS_MAP_REMOVABLE + REMOVABLE_STRIDE;

/// Where vanilla keeps them. Read only by `the_relocated_tables_match_vanilla`,
/// which is what stops [`REMOVABLE_PAIRS`] drifting from the bytes it replaces.
#[cfg(test)]
const MAP_REMOVABLE_VANILLA: usize = PRG012_FILE_BASE + 0x437;
#[cfg(test)]
const MAP_REMOVE_TO_VANILLA: usize = PRG012_FILE_BASE + 0x43F;

/// `CMP Map_Removable_Tiles,X` at `$A54C` and `LDA Map_RemoveTo_Tiles,X` at
/// `$A556` — the only two instructions in the ROM that name the tables. These
/// are their absolute operands.
const PRG012_REMOVABLE_OPERAND: usize = PRG012_FILE_BASE + 0x54D;
const PRG012_REMOVE_TO_OPERAND: usize = PRG012_FILE_BASE + 0x557;

/// `LDX #` at `$A54B`, which sizes vanilla's descending scan.
const PRG012_SCAN_COUNT: usize = PRG012_FILE_BASE + 0x54B;

/// Every obstacle the map can open, and what it opens into.
///
/// **The pairing is per tile byte, not per lock**, which is what makes an
/// obstacle *variant* cheap: several bytes may share a replacement (`$51` and
/// `$56` both reveal `$45`), so a new variant is a row here rather than a field
/// on every lock.
///
/// Two rules constrain a row, and vanilla's eight obey both:
///
/// * **The replacement must keep the tile's top two bits.** The map's palette
///   comes from those bits and the effect queues only pattern bytes — never an
///   attribute byte — so a pair that crossed pages would draw the revealed tile
///   in the obstacle's palette until the next map reload.
/// * **It must be walkable the way the corridor runs.** `Map_Object_Valid_*`
///   decides that; see `rom_data::gap_tile_for`, which picks the obstacle from
///   the path tile underneath for exactly this reason.
///
/// These are the base rows — terrain rather than choices: the rocks, the three
/// fortress variants, the water gap, and the three plain locks a lock wears when
/// its fortress is in the same world. [`obstacle_vocabulary`] adds the numbered
/// locks on top, and [`removable_rows`] picks the ones a given map earns.
#[rustfmt::skip]
pub(crate) const REMOVABLE_PAIRS: &[(u8, u8)] = &[
    (0x51, 0x45), // rock (horizontal) -> horizontal path
    (0x52, 0x46), // rock (vertical)   -> vertical path
    (0x54, 0x46), // lock (vertical)   -> vertical path
    (0x67, 0x60), // fortress          -> rubble
    (0xEB, 0xE3), // alt fortress      -> alt rubble
    (0xE4, 0xDA), // alt lock          -> sky path
    (0x56, 0x45), // lock (horizontal) -> horizontal path
    (0x9D, 0xB3), // river             -> bridge
    // --- past vanilla's eight -------------------------------------------
    //
    // `TILE_LARGEFORT`, which vanilla defines and never places. It has a
    // fortress's crumble sound and rubble in `prg011` (`:1823`, `:1832`) but no
    // completion path at all: `prg012`'s reload special-cases only `$67` and
    // `$EB`, and `$6A` was in neither table, so it took the threshold branch and
    // reloaded as a Mario/Luigi panel. This randomizer *does* place it
    // (`FORTRESS_TILES`), and [`ML_RANGE`]'s upper bound is what lets the row be
    // reached.
    (0x6A, 0x60), // large fortress    -> rubble
];

// --- Numbered locks -----------------------------------------------------

/// **A lock that says which world holds the fortress that opens it.**
///
/// A digit when the key is in another world, a plain lock when it is here — so
/// outside the maze, where every lock is local, not one of these tiles is ever
/// written.
///
/// **This replaced the map-object hint rather than joining it.** `maze::writer`
/// used to park a HELP bubble on every *local* lock, marking that set because it
/// was the smaller one and the nine per-world sprite slots could not afford the
/// other. A tile has no such budget, so the marked set can be the informative
/// one, and the sprite became a second way of saying strictly less. Its slots go
/// back to the map.
///
/// The property that carried over with it: **absence has to mean exactly one
/// thing.** A local lock left unmarked for want of a slot used to be
/// indistinguishable from a cross-world one, which is what
/// `lock_hint_slots_are_never_short` existed to prevent. Here it is
/// `the_obstacle_table_never_overflows` asserting that every away lock gets its
/// digit.
///
/// **Indexed `[orientation][world]`**, where orientation matches
/// [`HINT_REVEALS`]. The tile indices are the undefined tails that
/// [`ML_RANGE`]'s bounds released: `$6B-$7A` in page 1, `$EC-$F3` in page 3.
/// Page matters — it *is* the palette, and a row whose two tiles disagree about
/// it draws the revealed path in the lock's colors until the next map reload.
/// So the horizontal and vertical sets sit in page 1 with `$45`/`$46`, and the
/// sky set in page 3 with `$DA`, exactly as `$56` and `$E4` already do.
const HINT_TILES: [[u8; 8]; 4] = [
    [0x6B, 0x6C, 0x6D, 0x6E, 0x6F, 0x70, 0x71, 0x72], // horizontal
    [0x73, 0x74, 0x75, 0x76, 0x77, 0x78, 0x79, 0x7A], // vertical
    [0xEC, 0xED, 0xEE, 0xEF, 0xF0, 0xF1, 0xF2, 0xF3], // sky
    [0xF4, 0xF5, 0xF6, 0xF7, 0xF8, 0xF9, 0xFA, 0xFB], // water gap — see below
];

/// What each orientation's numbered lock reveals, and the plain lock it stands
/// in for. Same order as [`HINT_TILES`].
const HINT_REVEALS: [(u8, u8); 4] = [
    (0x56, 0x45), // horizontal lock -> horizontal path
    (0x54, 0x46), // vertical lock   -> vertical path
    (0xE4, 0xDA), // sky lock        -> sky path
    (0x9D, 0xB3), // water gap       -> bridge
];

/// **The water gap is the one variant that breaks the palette rule, on
/// purpose.**
///
/// A lock landing on a bridge wears `$9D`, which is page 2 — and page 2 has no
/// index to spare. Three tiles there are absent from every vanilla grid and all
/// three are traps: `$80` and `$81` are `TILE_MARIOCOMP_G`/`TILE_LUIGICOMP_G`,
/// the green completion panels the engine stamps at *runtime*, and `$B6` is a
/// single unexamined leftover. So the variant sits in page 3 with the sky set,
/// and reveals a page-2 bridge.
///
/// Two visible consequences, both accepted deliberately in exchange for the
/// hint reaching bridge locks at all:
///
/// * The lock draws in palette 3 rather than the palette 2 its water sits in.
/// * The revealed bridge keeps palette 3 until the next map reload, because the
///   effect queues pattern bytes and never an attribute byte. Leaving the map
///   and coming back fixes it.
///
/// Neither is a correctness problem — the tile byte written to the grid is
/// `$B3`, so the bridge is a bridge, walkable and persistent. It is only ever
/// the wrong color.
const WATER_ORIENTATION: usize = 3;

/// The level panels' lower-right quadrants for worlds 1-8: the digit glyphs.
///
/// **The whole art budget of this feature.** A variant is the tile it stands in
/// for with its lower-right quadrant swapped for one of these — so a path lock
/// stays a padlock and a bridge gap stays a river, each wearing a number. The
/// patterns are already in the bank, drawn by every numbered level on the map,
/// so no CHR is added and none of the 41 unreferenced patterns has to be
/// audited.
///
/// Pinned to the ROM by `the_digit_quadrants_are_the_panels_own`, which reads
/// them back out of metatiles `$03..$0A` rather than trusting this list.
const HINT_DIGITS: [u8; 8] = [0x8F, 0xA4, 0xA5, 0xA6, 0xA7, 0xC8, 0xC9, 0xCA];

/// Slots in the removable table, and so in the mirror.
///
/// **Fixed, and deliberately not "however many this seed needs".** Holding the
/// length constant keeps every scan count a compile-time constant — vanilla's
/// `LDX #` at `$A54B`, `FORTRESS_FX`'s, and `IS_COMPLETABLE`'s — so none of them
/// has to be patched per run and none can drift. Unused slots repeat row 0,
/// which is harmless: a duplicate can only be matched by a tile the original
/// already matches, and to the same replacement.
///
/// 24 is what the mirror's run holds: 149 bytes of `$FF` at [`FS_LOCK_MIRROR`]
/// to the end of PRG011, at six bytes an entry.
pub(crate) const REMOVABLE_COUNT: usize = 24;

/// Every obstacle tile that could ever be written, base rows and numbered locks
/// together.
///
/// This is the *vocabulary*, not the table. `is_completion_unsafe` asks about
/// tile bytes rather than about a particular seed, so it needs all of them; the
/// table written to the ROM holds only the ones a given map actually uses.
pub(crate) fn obstacle_vocabulary() -> Vec<(u8, u8)> {
    let mut out = REMOVABLE_PAIRS.to_vec();
    for (o, tiles) in HINT_TILES.iter().enumerate() {
        for &tile in tiles {
            out.push((tile, HINT_REVEALS[o].1));
        }
    }
    out
}

// --- The M/L range ------------------------------------------------------

/// `Tile_Attributes_TS0`, CPU `$A400` in PRG012: four thresholds indexed by the
/// tile's top two bits, `03 67 BF E9`, then the same four again at `+4`.
///
/// The duplication is not redundancy — the two rows are read from different
/// storage and answer different questions. `+0` is read straight from ROM by
/// the one site [`ML_RANGE`] replaces. `+4` is read through the RAM copy at
/// `$7E94` (filled per tileset by `prg030.asm:3597`) by four other sites: level
/// entry, the clear-FX selection, and two more. Nothing here touches `+4`.
const TILE_ATTRIBUTES_TS0: u16 = 0xA400;

/// Where the helper lands, and where its bound table lands inside it.
pub(crate) const ML_RANGE_CPU: u16 = prg_bank_file_to_cpu(12, FS_ML_RANGE);
const ML_RANGE_UPPER_CPU: u16 = ML_RANGE_CPU + 11;

/// The `CMP Tile_Attributes_TS0,X` at `$A545` that this replaces, and the `BCS`
/// after it. Five bytes; `JSR` + the same `BCS` is also five, and because the
/// `JSR` is the same length as the `CMP` the branch lands at the same address
/// and its operand does not change.
const PRG012_ML_TEST: usize = PRG012_FILE_BASE + 0x545;
/// What stands there in vanilla: `CMP $A400,X` / `BCS $A570`.
const PRG012_ML_TEST_VANILLA: [u8; 5] = [0xDD, 0x00, 0xA4, 0xB0, 0x26];

/// **The first tile of each page that is no longer flipped to an M/L marker.**
///
/// One past the last real tile in each page, so nothing vanilla places moves out
/// of the window: `$15` closes page 0's panels (including the nine authored
/// variants it never uses), `$6A` closes page 1 at the large fortress — which is
/// how that tile reaches the removable scan instead — `$BF` is page 2's only
/// entry, and page 3 ends at `$EB`, the alt fortress.
///
/// What falls outside is exactly the undefined tail of each page: `$16-$3F`,
/// `$6A-$7F`, `$EC-$FF`. Nothing is placed there today, which is why this
/// changes no build — and it is the whole point, because an obstacle tile has to
/// fall through to the removable scan rather than becoming a panel.
pub(crate) const ML_RANGE_UPPER: [u8; 4] = [0x16, 0x6A, 0xC0, 0xEC];

/// Is this completed tile flipped to a Mario/Luigi marker?
///
/// `A` is the tile and `X` its page (`tile >> 6`) on entry — both already in
/// hand at the call site. Carry is the answer, which is what lets a three-byte
/// `JSR` stand in for the three-byte `CMP` it replaces and leave the `BCS`
/// behind it untouched.
///
/// `X` and `Y` survive: the caller keeps its grid offset in `Y` across the call,
/// and `X` is the page it computed. Only `A` and the flags are spent.
///
/// 32 reserved, 15 used.
#[rustfmt::skip]
const ML_RANGE: [u8; 15] = [
    0xDD, ML_RANGE_UPPER_CPU as u8, (ML_RANGE_UPPER_CPU >> 8) as u8, //  0: CMP UPPER,X   ; past this page's window?
    0xB0, 0x04,                                                     //  3: BCS no
    0xDD, TILE_ATTRIBUTES_TS0 as u8, (TILE_ATTRIBUTES_TS0 >> 8) as u8, //  5: CMP Tile_Attributes_TS0,X
    0x60,                                                           //  8: RTS   ; carry IS the answer
    0x18,                                                           //  9: CLC   ; no
    0x60,                                                           // 10: RTS
    ML_RANGE_UPPER[0], ML_RANGE_UPPER[1], ML_RANGE_UPPER[2], ML_RANGE_UPPER[3], // 11: UPPER
];

/// The metatile quadrant tables are stored UL, LL, UR, LR — four 256-byte planes
/// from [`PRG012_FILE_BASE`]. The effect queues them in *row* order
/// (UL, UR, LL, LR), two per `Graphics_Buffer` run.
const PATTERN_QUADRANT_ORDER: [usize; 4] = [0, 2, 1, 3];

// --- The effect ---------------------------------------------------------

/// `MO_DoFortressFX`, rewritten to find its target by position instead of by
/// slot index.
///
/// Reached from the map-operation jump table with `Map_DoFortressFX` non-zero —
/// armed by the `(?)` orb, exactly as in vanilla — and with the player standing
/// on the fortress or map object they just cleared. The key is read straight off
/// `World_Map_Y`/`World_Map_X`/`World_Map_XHi`, so nothing hooks
/// `Map_MarkLevelComplete` and nothing depends on the tile under the player.
///
/// `$0743` and `$0744` carry the matched entry's target across the frames the
/// poof runs for. They are the two bytes the disassembly marks unused at
/// `$0743-$0744`, immediately below `Map_DoFortressFX` itself; nothing in the
/// ROM names them.
///
/// The order differs from vanilla's on purpose. Vanilla built the graphics
/// buffer first and only then discovered the lock was already busted, leaving a
/// queued VRAM write behind. Resolving the target cell first means the
/// removable-table index that decides the replacement tile is the same index
/// that picks the CHR quadrants, so it can stay in `X` from the lookup right
/// through the buffer build.
///
/// **Three guards are load-bearing and none is new.** The scan aborts when no
/// entry names the player's cell, which is every fortress that opens nothing.
/// The `find` loop aborts when the target cell holds something
/// `Map_Removable_Tiles` cannot name, which is the normal state of a lock a
/// hammer already opened. The `Map_Completions` test aborts when the bit is
/// already set, which is what stops map rows 7 and 8 — they share bit `$01`
/// (#212) — from opening each other's cells.
///
/// The visibility check between the map write and the buffer build is Fred's
/// algorithm, carried over from the patch this replaces. Its `±1` straddle
/// adjustment and `(col<<4) EOR $FD` edge filter are kept byte-for-byte: they
/// are about the *scroll*, not about the addressing being replaced, and three
/// in-house attempts to reason from position instead have already failed. What
/// did change is the half-screen index, which was six bytes of table lookup and
/// five of bit-shuffling to rebuild `2*screen + (col_in_screen >= 8)` from a
/// packed byte, and is now `col >> 3` — three `LSR`s on the raw column.
///
/// 537 reserved, 484 used.
#[rustfmt::skip]
const FORTRESS_FX: [u8; 484] = [
    0xA5, 0x20,                                    //   0: LDA <MAP_CLEAR_FX_CNT   ; mid-effect? the poof loop owns every later frame
    0xF0, 0x03,                                    //   2: BEQ chk_armed
    0x4C, 0x76, 0xC9,                              //   4: JMP poof
    0xAD, 0x45, 0x07,                              //   7: LDA MAP_DO_FX   ; the (?) orb armed this, exactly as in vanilla
    0xD0, 0x03,                                    //  10: BNE armed
    0x4C, 0xFD, 0xC7,                              //  12: JMP advance
    0xAE, PLAYER_CURRENT as u8, (PLAYER_CURRENT >> 8) as u8,                              //  15: LDX PLAYER_CURRENT
    0xB5, 0x75,                                    //  18: LDA <WORLD_MAP_Y,X
    0x29, 0xF0,                                    //  20: AND #$F0   ; (row+2)<<4 -- Map_Entered_Y's own encoding
    0x0D, WORLD_NUM as u8, (WORLD_NUM >> 8) as u8,                              //  22: ORA WORLD_NUM
    0x85, 0x00,                                    //  25: STA <T1   ; key0
    0xB5, 0x79,                                    //  27: LDA <WORLD_MAP_X,X
    0x29, 0xF0,                                    //  29: AND #$F0   ; col_in_screen<<4
    0x15, 0x77,                                    //  31: ORA <WORLD_MAP_XHI,X   ; | screen
    0x85, 0x01,                                    //  33: STA <T2   ; key1
    0xA2, 0x00,                                    //  35: LDX #$00   ; patched: (entries - 1) * 4
    0xBD, ENTRIES_CPU as u8, (ENTRIES_CPU >> 8) as u8,                              //  37: LDA ENTRIES,X
    0xC5, 0x00,                                    //  40: CMP <T1
    0xD0, 0x07,                                    //  42: BNE next_key
    0xBD, (ENTRIES_CPU + 1) as u8, ((ENTRIES_CPU + 1) >> 8) as u8,                              //  44: LDA ENTRIES+1,X
    0xC5, 0x01,                                    //  47: CMP <T2
    0xF0, 0x13,                                    //  49: BEQ hit
    0xCA,                                          //  51: DEX
    0xCA,                                          //  52: DEX
    0xCA,                                          //  53: DEX
    0xCA,                                          //  54: DEX
    0x10, 0xEC,                                    //  55: BPL find_key
    0xA9, 0x00,                                    //  57: LDA #$00
    0x8D, 0x45, 0x07,                              //  59: STA MAP_DO_FX
    0x85, 0x20,                                    //  62: STA <MAP_CLEAR_FX_CNT
    0xEE, 0x29, 0x07,                              //  64: INC MAP_OPERATION
    0x4C, 0x29, 0xCF,                              //  67: JMP WORLDMAP_UPD
    0xE0, 0x00,                                    //  70: CPX #$00   ; patched: away entries come first
    0xB0, 0x16,                                    //  72: BCS here
    0xBC, (ENTRIES_CPU + 2) as u8, ((ENTRIES_CPU + 2) >> 8) as u8,                              //  74: LDY ENTRIES+2,X   ; away: plane byte offset
    0xBD, (ENTRIES_CPU + 3) as u8, ((ENTRIES_CPU + 3) >> 8) as u8,                              //  77: LDA ENTRIES+3,X   ; away: bit mask
    0x48,                                          //  80: PHA
    0x19, PACKED as u8, (PACKED >> 8) as u8,                              //  81: ORA PACKED,Y
    0x99, PACKED as u8, (PACKED >> 8) as u8,                              //  84: STA PACKED,Y
    0x68,                                          //  87: PLA
    0x19, PACKED_MIRROR as u8, (PACKED_MIRROR >> 8) as u8,                              //  88: ORA PACKED_MIRROR,Y
    0x99, PACKED_MIRROR as u8, (PACKED_MIRROR >> 8) as u8,                              //  91: STA PACKED_MIRROR,Y
    0xD0, 0xD9,                                    //  94: BNE abort   ; A = the mask, never zero
    0xBD, (ENTRIES_CPU + 2) as u8, ((ENTRIES_CPU + 2) >> 8) as u8,                              //  96: LDA ENTRIES+2,X   ; home: (row+2)<<4 of the target
    0x8D, 0x43, 0x07,                              //  99: STA FX_ROW
    0xBD, (ENTRIES_CPU + 3) as u8, ((ENTRIES_CPU + 3) >> 8) as u8,                              // 102: LDA ENTRIES+3,X   ; home: the target's map column
    0x8D, 0x44, 0x07,                              // 105: STA FX_COL
    0xAD, 0x11, 0x07,                              // 108: LDA MAP_INTRO_TICK
    0xD0, 0x05,                                    // 111: BNE flash
    0xA9, 0x20,                                    // 113: LDA #$20
    0x8D, 0x11, 0x07,                              // 115: STA MAP_INTRO_TICK
    0x20, 0xD6, 0xC9,                              // 118: JSR FX_MONO_FLASH
    0xAD, 0x11, 0x07,                              // 121: LDA MAP_INTRO_TICK
    0xF0, 0x03,                                    // 124: BEQ begin
    0x4C, 0x29, 0xCF,                              // 126: JMP WORLDMAP_UPD
    0xA9, 0x80,                                    // 129: LDA #$80   ; SND_LEVELPOOF
    0x8D, 0xF2, 0x04,                              // 131: STA SOUND_Q1
    0xAD, 0x44, 0x07,                              // 134: LDA FX_COL
    0x4A,                                          // 137: LSR A
    0x4A,                                          // 138: LSR A
    0x4A,                                          // 139: LSR A
    0x29, 0x0E,                                    // 140: AND #$0E   ; screen * 2
    0xA8,                                          // 142: TAY
    0xB9, 0x00, 0x80,                              // 143: LDA TILE_MEM_ADDR,Y
    0x18,                                          // 146: CLC
    0x69, 0xF0,                                    // 147: ADC #$F0   ; +$110 once the row byte's +2 rows are added
    0x85, 0x0E,                                    // 149: STA <T15
    0xB9, 0x01, 0x80,                              // 151: LDA TILE_MEM_ADDR+1,Y
    0x69, 0x00,                                    // 154: ADC #$00
    0x85, 0x0F,                                    // 156: STA <T16
    0xAD, 0x43, 0x07,                              // 158: LDA FX_ROW
    0x29, 0xF0,                                    // 161: AND #$F0
    0x85, 0x0A,                                    // 163: STA <T11
    0xAD, 0x44, 0x07,                              // 165: LDA FX_COL
    0x29, 0x0F,                                    // 168: AND #$0F
    0x05, 0x0A,                                    // 170: ORA <T11
    0xA8,                                          // 172: TAY   ; Y = the cell's index in its screen's page
    0xB1, 0x0E,                                    // 173: LDA [T15],Y   ; the tile standing there now
    0xA2, (REMOVABLE_COUNT - 1) as u8,             // 175: LDX #(entries - 1)
    0xDD, MIRROR_REMOVABLE as u8, (MIRROR_REMOVABLE >> 8) as u8,                              // 177: CMP MIRROR_REMOVABLE,X
    0xF0, 0x06,                                    // 180: BEQ found
    0xCA,                                          // 182: DEX
    0x10, 0xF8,                                    // 183: BPL find
    0x4C, 0xF6, 0xC7,                              // 185: JMP abort   ; not a gap: the cell is already open
    0x86, 0x0B,                                    // 188: STX <T12   ; the removable-table index drives everything
    0xAD, 0x43, 0x07,                              // 190: LDA FX_ROW
    0x29, 0x0F,                                    // 193: AND #$0F   ; completion row, 0-7 (rows 7 and 8 share)
    0xAA,                                          // 195: TAX
    0xBD, MAP_COMPLETE_BIT_CPU as u8, (MAP_COMPLETE_BIT_CPU >> 8) as u8,                              // 196: LDA MAP_COMPLETE_BIT,X
    0x85, 0x0A,                                    // 199: STA <T11
    0xAE, 0x44, 0x07,                              // 201: LDX FX_COL
    0xBD, MAP_COMPLETIONS as u8, (MAP_COMPLETIONS >> 8) as u8,                              // 204: LDA MAP_COMPLETIONS,X
    0x25, 0x0A,                                    // 207: AND <T11
    0xF0, 0x03,                                    // 209: BEQ fresh
    0x4C, 0xF6, 0xC7,                              // 211: JMP abort   ; already busted
    0xBD, MAP_COMPLETIONS as u8, (MAP_COMPLETIONS >> 8) as u8,                              // 214: LDA MAP_COMPLETIONS,X
    0x05, 0x0A,                                    // 217: ORA <T11
    0x9D, MAP_COMPLETIONS as u8, (MAP_COMPLETIONS >> 8) as u8,                              // 219: STA MAP_COMPLETIONS,X
    0xBD, (MAP_COMPLETIONS + 64) as u8, ((MAP_COMPLETIONS + 64) >> 8) as u8,                              // 222: LDA MAP_COMPLETIONS+64,X   ; the other player, so a game over cannot undo it
    0x05, 0x0A,                                    // 225: ORA <T11
    0x9D, (MAP_COMPLETIONS + 64) as u8, ((MAP_COMPLETIONS + 64) >> 8) as u8,                              // 227: STA MAP_COMPLETIONS+64,X
    0xA6, 0x0B,                                    // 230: LDX <T12
    0xBD, MIRROR_REMOVE_TO as u8, (MIRROR_REMOVE_TO >> 8) as u8,                              // 232: LDA MIRROR_REMOVE_TO,X
    0x91, 0x0E,                                    // 235: STA [T15],Y
    0xAD, 0x44, 0x07,                              // 237: LDA FX_COL
    0x29, 0x0F,                                    // 240: AND #$0F
    0x0A,                                          // 242: ASL A
    0x0A,                                          // 243: ASL A
    0x0A,                                          // 244: ASL A
    0x0A,                                          // 245: ASL A
    0x45, 0xFD,                                    // 246: EOR <MAP_SCROLL_X
    0xC9, 0x10,                                    // 248: CMP #$10
    0x90, 0x3A,                                    // 250: BCC invisible
    0xC9, 0xE8,                                    // 252: CMP #$E8
    0xB0, 0x36,                                    // 254: BCS invisible
    0xAD, 0x44, 0x07,                              // 256: LDA FX_COL
    0x4A,                                          // 259: LSR A
    0x4A,                                          // 260: LSR A
    0x4A,                                          // 261: LSR A   ; half-screen index = 2*screen + (col>=8)
    0x85, 0x0A,                                    // 262: STA <T11
    0xA5, 0x79,                                    // 264: LDA <MARIO_X
    0x0A,                                          // 266: ASL A
    0xA5, 0x77,                                    // 267: LDA <MARIO_XHI
    0x65, 0x77,                                    // 269: ADC <MARIO_XHI
    0x85, 0x0B,                                    // 271: STA <T12
    0xC5, 0x0A,                                    // 273: CMP <T11
    0xF0, 0x12,                                    // 275: BEQ visible
    0xA5, 0x79,                                    // 277: LDA <MARIO_X
    0x45, 0xFD,                                    // 279: EOR <MAP_SCROLL_X
    0x30, 0x04,                                    // 281: BMI adj_up
    0xC6, 0x0A,                                    // 283: DEC <T11
    0xC6, 0x0A,                                    // 285: DEC <T11
    0xE6, 0x0A,                                    // 287: INC <T11
    0xA5, 0x0B,                                    // 289: LDA <T12
    0xC5, 0x0A,                                    // 291: CMP <T11
    0xD0, 0x0F,                                    // 293: BNE invisible
    0xA9, 0x01,                                    // 295: LDA #$01
    0x85, 0x20,                                    // 297: STA <MAP_CLEAR_FX_CNT
    0xAC, 0x98, 0x05,                              // 299: LDY WORLD_8_DARK
    0xF0, 0x0D,                                    // 302: BEQ build
    0x8D, 0x98, 0x05,                              // 304: STA WORLD_8_DARK   ; A=1: replay the arrival reveal
    0x4C, 0x76, 0xC9,                              // 307: JMP poof
    0xA9, 0x06,                                    // 310: LDA #$06
    0x85, 0x20,                                    // 312: STA <MAP_CLEAR_FX_CNT
    0x4C, 0x76, 0xC9,                              // 314: JMP poof
    0xAC, 0x00, 0x03,                              // 317: LDY GBUFCNT
    0xA9, 0x00,                                    // 320: LDA #$00
    0x85, 0x0A,                                    // 322: STA <T11
    0xAD, 0x43, 0x07,                              // 324: LDA FX_ROW
    0x29, 0xF0,                                    // 327: AND #$F0
    0x0A,                                          // 329: ASL A
    0x26, 0x0A,                                    // 330: ROL <T11
    0x0A,                                          // 332: ASL A
    0x26, 0x0A,                                    // 333: ROL <T11
    0x85, 0x0B,                                    // 335: STA <T12
    0xAD, 0x44, 0x07,                              // 337: LDA FX_COL
    0x29, 0x0F,                                    // 340: AND #$0F
    0x0A,                                          // 342: ASL A
    0x65, 0x0B,                                    // 343: ADC <T12
    0x85, 0x0B,                                    // 345: STA <T12
    0xA5, 0x0A,                                    // 347: LDA <T11
    0x69, 0x28,                                    // 349: ADC #$28   ; vram = $2800 + row_byte*4 + col_in_screen*2
    0x85, 0x0A,                                    // 351: STA <T11
    0x99, 0x01, 0x03,                              // 353: STA GBUF,Y
    0xC8,                                          // 356: INY
    0xA5, 0x0B,                                    // 357: LDA <T12
    0x99, 0x01, 0x03,                              // 359: STA GBUF,Y
    0xC8,                                          // 362: INY
    0xA9, 0x02,                                    // 363: LDA #$02
    0x99, 0x01, 0x03,                              // 365: STA GBUF,Y
    0xC8,                                          // 368: INY
    0x8A,                                          // 369: TXA
    0x0A,                                          // 370: ASL A
    0x0A,                                          // 371: ASL A
    0xAA,                                          // 372: TAX   ; four quadrants per removable entry
    0xBD, MIRROR_PATTERNS as u8, (MIRROR_PATTERNS >> 8) as u8,                              // 373: LDA MIRROR_PATTERNS,X
    0x99, 0x01, 0x03,                              // 376: STA GBUF,Y
    0xE8,                                          // 379: INX
    0xC8,                                          // 380: INY
    0xBD, MIRROR_PATTERNS as u8, (MIRROR_PATTERNS >> 8) as u8,                              // 381: LDA MIRROR_PATTERNS,X
    0x99, 0x01, 0x03,                              // 384: STA GBUF,Y
    0xE8,                                          // 387: INX
    0xC8,                                          // 388: INY
    0xA5, 0x0B,                                    // 389: LDA <T12
    0x18,                                          // 391: CLC
    0x69, 0x20,                                    // 392: ADC #32   ; the metatile's bottom half, one row down
    0x85, 0x0B,                                    // 394: STA <T12
    0xA5, 0x0A,                                    // 396: LDA <T11
    0x69, 0x00,                                    // 398: ADC #$00
    0x85, 0x0A,                                    // 400: STA <T11
    0x99, 0x01, 0x03,                              // 402: STA GBUF,Y
    0xC8,                                          // 405: INY
    0xA5, 0x0B,                                    // 406: LDA <T12
    0x99, 0x01, 0x03,                              // 408: STA GBUF,Y
    0xC8,                                          // 411: INY
    0xA9, 0x02,                                    // 412: LDA #$02
    0x99, 0x01, 0x03,                              // 414: STA GBUF,Y
    0xC8,                                          // 417: INY
    0xBD, MIRROR_PATTERNS as u8, (MIRROR_PATTERNS >> 8) as u8,                              // 418: LDA MIRROR_PATTERNS,X
    0x99, 0x01, 0x03,                              // 421: STA GBUF,Y
    0xC8,                                          // 424: INY
    0xE8,                                          // 425: INX
    0xBD, MIRROR_PATTERNS as u8, (MIRROR_PATTERNS >> 8) as u8,                              // 426: LDA MIRROR_PATTERNS,X
    0x99, 0x01, 0x03,                              // 429: STA GBUF,Y
    0xC8,                                          // 432: INY
    0xA9, 0x00,                                    // 433: LDA #$00
    0x99, 0x01, 0x03,                              // 435: STA GBUF,Y
    0x8C, 0x00, 0x03,                              // 438: STY GBUFCNT
    0xA5, 0x15,                                    // 441: LDA <COUNTER_1
    0x29, 0x03,                                    // 443: AND #$03
    0xD0, 0x0B,                                    // 445: BNE draw
    0xE6, 0x20,                                    // 447: INC <MAP_CLEAR_FX_CNT
    0xA5, 0x20,                                    // 449: LDA <MAP_CLEAR_FX_CNT
    0xC9, 0x07,                                    // 451: CMP #$07
    0xD0, 0x03,                                    // 453: BNE draw
    0x4C, 0xF6, 0xC7,                              // 455: JMP abort
    0xAD, 0x43, 0x07,                              // 458: LDA FX_ROW
    0x29, 0xF0,                                    // 461: AND #$F0
    0x85, 0x00,                                    // 463: STA <T1
    0xAD, 0x44, 0x07,                              // 465: LDA FX_COL
    0x29, 0x0F,                                    // 468: AND #$0F
    0x0A,                                          // 470: ASL A
    0x0A,                                          // 471: ASL A
    0x0A,                                          // 472: ASL A
    0x0A,                                          // 473: ASL A
    0x85, 0x01,                                    // 474: STA <T2
    0xA4, 0x20,                                    // 476: LDY <MAP_CLEAR_FX_CNT
    0x20, 0xCF, 0xAB,                              // 478: JSR MAP_DRAW_POOF
    0x4C, 0x29, 0xCF,                              // 481: JMP WORLDMAP_UPD
];

// --- The table ----------------------------------------------------------

/// One lock and the fortress that opens it.
///
/// Positions are grid `(row, col)` — the same coordinates the map grids and
/// `pipe_helpers::grid_pos_to_dest_nibbles` use. A pair whose two worlds match
/// is a *home* lock (animated where the player stands); a pair that straddles
/// worlds is an *away* lock (a completion bit set into the packed store).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LockEntry {
    /// The world the fortress stands in, 0-based. Where the player will be.
    pub key_world: usize,
    /// The fortress's map cell.
    pub key_pos: (usize, usize),
    /// The world the lock stands in, 0-based.
    pub target_world: usize,
    /// The lock's map cell.
    pub target_pos: (usize, usize),
}

impl LockEntry {
    fn is_away(&self) -> bool {
        self.key_world != self.target_world
    }

    /// `[key0, key1]` — the player's own cell, in the form the live map
    /// variables already hold it.
    ///
    /// `key0` is `World_Map_Y & $F0` (which is `(grid row + 2) << 4`) with the
    /// world in the free low nibble; `key1` is `World_Map_X & $F0` (the column
    /// within the screen) with `World_Map_XHi` (the screen) in its low nibble.
    /// The console builds both in nine instructions with no table lookup, and
    /// every grid row stays distinct — unlike `Map_MarkLevelComplete`'s
    /// completion index, which folds rows 7 and 8 together.
    fn key(&self) -> [u8; 2] {
        let (row, col) = self.key_pos;
        debug_assert!(self.key_world < 8, "a world index is three bits");
        debug_assert!(row <= 8, "a map grid has nine rows");
        debug_assert!(col < HALF_LEN, "a map column is six bits");
        [
            (((row + 2) as u8) << 4) | self.key_world as u8,
            (((col % 16) as u8) << 4) | (col / 16) as u8,
        ]
    }
}

/// `[t0, t1]` for a home lock: the row byte the effect wants, and the column.
///
/// `t0` packs both facts about the target row that the effect needs and neither
/// of which is derivable from the other cheaply. The high nibble is
/// `(row + 2) << 4` — the same `Map_Entered_Y` encoding, `OR`ed straight into
/// the map-data write index — and the low nibble is the completion row, which
/// indexes `Map_CompleteBit`. Deriving the second from the first on the console
/// costs a clamp; deriving it here costs nothing, and the nibble was free
/// because the engine only ever reads the high half.
fn home_target(pos: (usize, usize)) -> [u8; 2] {
    let (row, col) = pos;
    debug_assert!(row <= 8, "a map grid has nine rows");
    debug_assert!(col < HALF_LEN, "a completion column is six bits");
    [(((row + 2) as u8) << 4) | row.min(7) as u8, col as u8]
}

/// `[t0, t1]` for an away lock: where its bit sits in the packed store, as
/// `(byte offset from `PACKED`, single-bit mask)`.
///
/// **Derived by running the packer**, not by re-deriving its arithmetic: a
/// `Map_Completions` half with exactly this cell's bit set is packed for the
/// destination world, and whatever bit comes out the other side is the answer.
/// The bit order, the row 7/8 fold and the stencil are then the packer's by
/// construction, and cannot drift from it.
///
/// `None` when the cell owns no bit at all — the stencil says the engine can
/// never act on it, so a lock there could not be persisted by any means.
fn away_target(map: &CompletionMap, world: usize, pos: (usize, usize)) -> Option<[u8; 2]> {
    let (row, col) = pos;
    if col >= HALF_LEN {
        return None;
    }
    let mut half = [0u8; HALF_LEN];
    half[col] = MAP_COMPLETE_BITS[row.min(7)];

    let plane = map.pack(world, &half);
    let byte = plane.iter().position(|&b| b != 0)?;
    let mask = plane[byte];
    debug_assert_eq!(mask.count_ones(), 1, "one cell must pack to exactly one bit");

    let offset = map.base(world) + byte;
    debug_assert!(offset < PLANE_RESERVE, "a plane offset outside the reserve is another world's");
    Some([offset as u8, mask])
}

/// Move `Map_Removable_Tiles` / `Map_RemoveTo_Tiles` out of the wall they are
/// built into, and point vanilla's scan at the copy.
///
/// At the vanilla address the next byte belongs to `Map_Completable_Tiles`, so
/// the table could not gain an entry without overwriting a live one. Here it
/// can, and `rows` is what goes in it.
///
/// Three writes. The two tables (into a fixed stride, so the second never has to
/// move again), then the two absolute operands at `$A54D` and `$A557`, then the
/// `LDX #` at `$A54B` that sizes the descending scan.
///
/// Idempotent: `rows` is derived from the map, never from the table, so running
/// twice is running once. Vanilla's bytes are deliberately left where they are —
/// nothing reads them afterwards, and leaving them makes the relocation
/// auditable against an unpatched ROM.
pub(crate) fn relocate_removable_tables(rom: &mut Rom, rows: &[(u8, u8)]) {
    assert_eq!(rows.len(), REMOVABLE_COUNT, "the table is a fixed {REMOVABLE_COUNT} slots");
    const {
        assert!(
            REMOVABLE_COUNT <= REMOVABLE_STRIDE,
            "more removable entries than the stride reserves: the second table would be \
             overwritten. Raise REMOVABLE_STRIDE and the allocation with it."
        )
    };
    for (i, &(from, to)) in rows.iter().enumerate() {
        rom.write_byte(MAP_REMOVABLE_TILES + i, from);
        rom.write_byte(MAP_REMOVE_TO_TILES + i, to);
    }

    let removable_cpu = prg_bank_file_to_cpu(12, MAP_REMOVABLE_TILES);
    let remove_to_cpu = prg_bank_file_to_cpu(12, MAP_REMOVE_TO_TILES);
    rom.write_range(PRG012_REMOVABLE_OPERAND, &removable_cpu.to_le_bytes());
    rom.write_range(PRG012_REMOVE_TO_OPERAND, &remove_to_cpu.to_le_bytes());
    rom.write_byte(PRG012_SCAN_COUNT, (REMOVABLE_COUNT - 1) as u8);

    install_ml_range(rom);
}

/// Which of the 256 tile bytes appear anywhere on the eight finished maps.
///
/// The single producer of that question. Two features ask it — the removable
/// table and the hammer's breakable table — and they must agree, because a tile
/// one of them knows about and the other does not is a lock that one thing can
/// open and another cannot.
pub(crate) fn tiles_on_map(rom: &Rom) -> [bool; 256] {
    let mut present = [false; 256];
    for world in 0..8 {
        let info = &rom_data::MAP_TILE_GRIDS[world];
        for screen in 0..info.screens {
            for row in 0..9 {
                for col in 0..16 {
                    let off = rom_data::map_tile_offset(world, row, screen * 16 + col);
                    present[rom.read_byte(off) as usize] = true;
                }
            }
        }
    }
    present
}

/// A numbered lock's `(revealed tile, break-animation index)`, or `None` if this
/// is not one.
///
/// The animation index is the hammer's, and the vertical set is the odd one out
/// — the same `1, 0, 0` the plain locks use, for the same reason.
pub(crate) fn numbered_lock(tile: u8) -> Option<(u8, u8)> {
    HINT_TILES.iter().position(|set| set.contains(&tile)).map(|orientation| {
        (HINT_REVEALS[orientation].1, u8::from(orientation == VERTICAL_ORIENTATION))
    })
}

/// Is this numbered lock a water gap rather than a path lock?
///
/// The hammer asks, because the two are governed by different options: a bridge
/// gap is broken under "hammer breaks bridges", a path lock under "hammer breaks
/// locks", and giving a bridge gap a digit must not quietly move it from one
/// switch to the other.
pub(crate) fn numbered_lock_is_water(tile: u8) -> bool {
    HINT_TILES[WATER_ORIENTATION].contains(&tile)
}

/// The index of the vertical set in [`HINT_TILES`] / [`HINT_REVEALS`].
const VERTICAL_ORIENTATION: usize = 1;

/// **A row for every obstacle actually standing on the map, and nothing else.**
///
/// The vocabulary is larger than the table — 33 possible obstacles against 24
/// slots — and that is fine, because no single map can wear more than a few of
/// it. The bound is not a hope:
///
/// * **Terrain: 6 rows.** Two rocks, three fortress variants, and the water gap
///   — `$9D` is a vertical river segment and ordinary scenery, 45 cells on a
///   water-heavy map, so its row is always spoken for whether or not any lock
///   sits on a bridge.
/// * **Every lock contributes at most one row.** However many home locks there
///   are, they share the plain rows between them; each away lock adds its
///   `(world, orientation)` variant, and locks agreeing on both share one. A
///   build pairs its 17 fortresses with 17 locks — measured 16 or 17 across
///   every maze seed sampled.
///
/// 6 + 17 = 23, and seed 37 reaches exactly that. One slot spare, which is
/// enough because both terms are counts of things the builder fixes, not of
/// things that grow with map size.
///
/// The assert is the guard if either term ever moves — `MAX_ENTRIES` permits 28
/// locks, and at 19 this would overflow. It fails the build loudly, which is the
/// right failure: a truncated table leaves a lock no fortress can open.
///
/// Reading it off the finished grids rather than tracking it through placement
/// means the table describes the map that shipped, not the map we intended.
pub(crate) fn removable_rows(rom: &Rom) -> Vec<(u8, u8)> {
    let present = tiles_on_map(rom);

    let mut rows: Vec<(u8, u8)> =
        obstacle_vocabulary().into_iter().filter(|&(tile, _)| present[tile as usize]).collect();
    assert!(
        rows.len() <= REMOVABLE_COUNT,
        "{} obstacles on the map but only {REMOVABLE_COUNT} table slots — some lock would \
         never open. See `removable_rows` for why this was thought impossible.",
        rows.len()
    );

    // Unused slots repeat the first row rather than sitting as `$FF`: the scan
    // is sized by a constant and walks every slot, and `$FF` is a real tile.
    let pad = *rows.first().unwrap_or(&REMOVABLE_PAIRS[0]);
    rows.resize(REMOVABLE_COUNT, pad);
    rows
}

/// Give every away lock a numbered tile, and define the metatiles it needs.
///
/// An away lock is one whose fortress stands in another world, which outside the
/// maze never happens — so this is a no-op for an ordinary seed, and not one
/// numbered tile is defined.
///
/// The digit is the *fortress's* world, because that is the question the player
/// is asking: not where am I, but where do I have to go. A local lock keeps the
/// plain tile, and the absence of a digit is itself the answer.
fn stamp_numbered_locks(rom: &mut Rom, entries: &[LockEntry]) {
    for e in entries.iter().filter(|e| e.is_away()) {
        let off = rom_data::map_tile_offset(e.target_world, e.target_pos.0, e.target_pos.1);
        let plain = rom.read_byte(off);
        let Some(orientation) = HINT_REVEALS.iter().position(|&(lock, _)| lock == plain) else {
            // Either this function run twice, or a pairing bug. Neither
            // should pass quietly.
            debug_assert!(
                HINT_TILES.iter().any(|set| set.contains(&plain)),
                "away lock at {:?} in W{} wears {plain:#04X}, which is no lock tile",
                e.target_pos,
                e.target_world + 1
            );
            continue;
        };
        // Keyed by the number shown, not the internal index, so a tile means the
        // same thing in every seed: `$6B` is always "horizontal, world 1".
        let shown = displayed_world(rom, e.key_world);
        let tile = HINT_TILES[orientation][shown - 1];
        rom.write_byte(off, tile);
        write_hint_metatile(rom, tile, plain, shown - 1);
    }
}

/// The world number the *player* sees for an internal world index.
///
/// **These are two different facts, and the maze guarantees they differ.** World
/// order shuffles which map is reached when, and world-maze forces it on;
/// `world_order` then rewrites both "WORLD X" display sites to read an
/// internal → display-tile table instead of `World_Num` itself. A hint that
/// showed the internal index would name a world whose number the player has
/// never seen.
///
/// The tile is `$F0 | number`. Anything else means the table was never written —
/// world order off, which cannot happen alongside a numbered lock today — and
/// the vanilla identity is the right answer there.
fn displayed_world(rom: &Rom, internal: usize) -> usize {
    let tile = rom.read_byte(super::world_order::DISPLAY_TABLE_OFFSET + internal);
    if (0xF1..=0xF8).contains(&tile) { (tile & 0x0F) as usize } else { internal + 1 }
}

/// Define one numbered lock's metatile: the tile it stands in for, wearing a
/// digit in its lower-right quadrant.
///
/// **The art is copied from `plain` rather than written out**, which is what
/// keeps each obstacle looking like itself: a path lock stays a padlock, and a
/// bridge gap stays a river with a number on it rather than becoming a padlock
/// floating in the water. It also means the three quadrants are never a second
/// copy of bytes the ROM already holds.
///
/// The quadrant planes are stored **UL, LL, UR, LR** — not in row order — so
/// planes 0-2 are the three that carry over and plane 3 is the corner the digit
/// takes. Same trap [`PATTERN_QUADRANT_ORDER`] exists for on the read side.
fn write_hint_metatile(rom: &mut Rom, tile: u8, plain: u8, world: usize) {
    for plane in 0..3 {
        let pattern = rom.read_byte(PRG012_FILE_BASE + plane * 256 + plain as usize);
        rom.write_byte(PRG012_FILE_BASE + plane * 256 + tile as usize, pattern);
    }
    rom.write_byte(PRG012_FILE_BASE + 3 * 256 + tile as usize, HINT_DIGITS[world]);
}

/// Bound the top of each page's M/L range, and point the reload's test at the
/// helper that does it.
///
/// The splice is the same length as what it replaces, so the `BCS` behind it
/// keeps both its address and its operand — this is a three-byte instruction
/// swapped for another three-byte instruction, not a relocation.
///
/// Idempotent, and it reads nothing from the ROM.
fn install_ml_range(rom: &mut Rom) {
    rom.write_range(FS_ML_RANGE, &ML_RANGE);
    let mut splice = [0u8; 5];
    splice[0] = 0x20; // JSR
    splice[1..3].copy_from_slice(&ML_RANGE_CPU.to_le_bytes());
    splice[3..].copy_from_slice(&PRG012_ML_TEST_VANILLA[3..]); // the same BCS
    rom.write_range(PRG012_ML_TEST, &splice);
}

/// The 48 bytes of PRG012 the effect cannot reach: the removable-tile pairing
/// and the CHR quadrants of each tile it produces.
///
/// One index into all three, which is the point — the tile the effect writes and
/// the patterns it draws stop being independent facts that happen to agree.
fn mirror_bytes(rom: &Rom) -> [u8; MIRROR_LEN] {
    let mut out = [0u8; MIRROR_LEN];
    for i in 0..REMOVABLE_COUNT {
        let from = rom.read_byte(MAP_REMOVABLE_TILES + i);
        let to = rom.read_byte(MAP_REMOVE_TO_TILES + i);
        out[i] = from;
        out[REMOVABLE_COUNT + i] = to;
        for (j, q) in PATTERN_QUADRANT_ORDER.iter().enumerate() {
            out[2 * REMOVABLE_COUNT + i * 4 + j] =
                rom.read_byte(PRG012_FILE_BASE + q * 256 + to as usize);
        }
    }
    out
}

/// The Rust copy of the gap-tile mapping, checked against the mirror it will now
/// share a ROM with.
///
/// `rom_data::path_for_gap_tile` is what the *builder* uses to decide what a
/// lock covers; the mirror is what the *console* uses to decide what it reveals.
/// They were independent before this rework and had to agree by inspection. Now
/// they are checked, every run, for free.
fn assert_mirror_agrees_with_rust(mirror: &[u8; MIRROR_LEN]) {
    // Only the gap tiles this map actually wears: the table now carries a row
    // per obstacle present, so a seed with no water gap has no `$9D` row and
    // there is nothing to agree about.
    for gap in [0x54u8, 0x56, 0xE4, rom_data::WATER_GAP_TILE] {
        if !mirror[..REMOVABLE_COUNT].contains(&gap) {
            continue;
        }
        let from_mirror =
            (0..REMOVABLE_COUNT).find(|&i| mirror[i] == gap).map(|i| mirror[REMOVABLE_COUNT + i]);
        assert_eq!(
            rom_data::path_for_gap_tile(gap),
            from_mirror,
            "gap tile {gap:#04X}: `path_for_gap_tile` disagrees with `Map_RemoveTo_Tiles`"
        );
    }
}

/// **A fortress opens one lock, and a lock has one fortress.**
///
/// The table is a partial one-to-one map between completable cells: at most one
/// entry per key, at most one per target. Both halves come from the same rule —
/// *a lock breaking is the only feedback that says which fortress did it* — so a
/// fortress that opened two would be unreadable, and a lock with two keys is a
/// gate that is not really a gate. Measured: every build pairs its 17 fortresses
/// with 17 locks, one each.
///
/// **Both producers guarantee it, and neither is checked by the other.** The
/// overworld builder's pairing is injective because no two locks in a world
/// share a fortress section; `maze::fill` preserves that because its only move
/// swaps two locks' forts. What broke it was *assembly* — taking the maze's
/// cross-world half and the builder's local half and adding them together, which
/// discarded every swap that left both locks at home (33.1% of them) and let a
/// fortress keep a stale local lock while gaining a foreign one. That is exactly
/// the kind of failure this catches: no crash, no wrong tile, just a map whose
/// gates are not the ones anything verified.
fn assert_one_key_per_lock(entries: &[LockEntry]) {
    let mut by_key: HashMap<(usize, (usize, usize)), &LockEntry> = HashMap::new();
    let mut by_target: HashMap<(usize, (usize, usize)), &LockEntry> = HashMap::new();
    for e in entries {
        if let Some(prev) = by_key.insert((e.key_world, e.key_pos), e) {
            panic!(
                "fortress {:?} in world {} opens both {prev:?} and {e:?}",
                e.key_pos, e.key_world
            );
        }
        if let Some(prev) = by_target.insert((e.target_world, e.target_pos), e) {
            panic!(
                "lock {:?} in world {} is opened by both {prev:?} and {e:?}",
                e.target_pos, e.target_world
            );
        }
    }
}

/// Install the position-keyed lock mechanism: the mirror, the rewritten effect,
/// the jump-table repoint and the entry table.
///
/// Call it **after** the overworld writer has laid down the final map grids and
/// with the same ROM state [`super::completion_bits::apply`] sees — the packed
/// layout is derived from those grids, so a table built against a different
/// version of them names the wrong bits.
///
/// **Nothing here touches enemy data.** The `(?)` orb still arms the effect and
/// all 17 of vanilla's Boom-Booms already carry a non-zero Y-nibble, so the
/// trigger needs no help. An earlier cut of this rework zeroed those nibbles,
/// which only made sense while the orb path was being retired.
///
/// # Panics
///
/// If [`assert_one_key_per_lock`] is violated, or if more than [`MAX_ENTRIES`]
/// are handed over. Truncating would leave a lock no fortress opens, which is an
/// unwinnable seed; a build-time failure is the better end of that trade.
pub fn apply(rom: &mut Rom, entries: &[LockEntry]) {
    assert_one_key_per_lock(entries);

    // Order matters and is one-way. The numbered tiles are stamped onto the map
    // first, because `removable_rows` reads the map to decide what the table
    // needs; the table is written next, because `mirror_bytes` reads the table.
    stamp_numbered_locks(rom, entries);
    relocate_removable_tables(rom, &removable_rows(rom));

    let mirror = mirror_bytes(rom);
    assert_mirror_agrees_with_rust(&mirror);
    rom.write_range(FS_LOCK_MIRROR, &mirror);

    // Away entries first: the scan's only test is `CPX #boundary` against its
    // own descending index, so the split has to be positional.
    let mut table: Vec<u8> = Vec::with_capacity(entries.len() * ENTRY_LEN);
    let away: Vec<&LockEntry> = entries.iter().filter(|e| e.is_away()).collect();
    let mut away_count = 0usize;
    if !away.is_empty() {
        let map = CompletionMap::from_rom(rom);

        // **The ordering guard, and it catches three rules at once.**
        //
        // This module derives each away lock's `(plane byte, bit)` by re-reading
        // the map grids, while `completion_bits` has already emitted a base
        // table from the grids as IT saw them. Those two views agree only if the
        // grids have not moved in between — so comparing them here catches, in
        // four lines: a grid write landing after `completion_bits::apply` ran;
        // this module running BEFORE it; and `completion_bits` never having run
        // at all, in which case the region is still `$FF` filler and the compare
        // fails loudly. Every one of those is otherwise silent.
        let emitted = rom.read_range(FS_COMPLETION_BASES, 9);
        assert_eq!(
            emitted,
            map.base_table(),
            "the packed-store base table in the ROM disagrees with the map this module just \
             read. Either a grid was written after `completion_bits::apply`, or this ran before \
             it, or it never ran. See `randomizer::randomize_inner` for the order."
        );

        for e in away {
            let Some(target) = away_target(&map, e.target_world, e.target_pos) else {
                debug_assert!(false, "lock cell {:?} owns no completion bit", e.target_pos);
                continue;
            };
            table.extend_from_slice(&e.key());
            table.extend_from_slice(&target);
            away_count += 1;
        }
    }
    for e in entries.iter().filter(|e| !e.is_away()) {
        table.extend_from_slice(&e.key());
        table.extend_from_slice(&home_target(e.target_pos));
    }

    let count = table.len() / ENTRY_LEN;
    assert!(count <= MAX_ENTRIES, "{count} locks but only {MAX_ENTRIES} entries fit the table");

    let mut code = FORTRESS_FX;
    // The scan walks backwards from the last entry's first byte, so the loop
    // tail is `DEX x4 / BPL` rather than a compare against a length. With no
    // entries at all it reads index 0, which is `$FF` filler and matches no
    // key — the routine then clears and advances, which is correct.
    code[COUNT_OPERAND] = (count.saturating_sub(1) * ENTRY_LEN) as u8;
    code[BOUNDARY_OPERAND] = (away_count * ENTRY_LEN) as u8;
    rom.write_range(FS_FORTRESS_FX, &code);
    rom.write_range(MAP_OP8_VECTOR, &FORTRESS_FX_CPU.to_le_bytes());
    if !table.is_empty() {
        rom.write_range(FS_LOCK_ENTRIES, &table);
    }
}

/// Decode the entry table back out of a finished ROM.
///
/// The layout knowledge lives here rather than in the tests that read it, so a
/// change to the entry shape cannot leave a test quietly decoding the old one.
#[cfg(test)]
pub(crate) fn decode_entries(rom: &Rom) -> Vec<DecodedEntry> {
    if rom.read_range(MAP_OP8_VECTOR, 2) != FORTRESS_FX_CPU.to_le_bytes() {
        return Vec::new();
    }
    let count = rom.read_byte(FS_FORTRESS_FX + COUNT_OPERAND) as usize / ENTRY_LEN + 1;
    let boundary = rom.read_byte(FS_FORTRESS_FX + BOUNDARY_OPERAND) as usize;
    (0..count)
        .map(|i| {
            let b = FS_LOCK_ENTRIES + i * ENTRY_LEN;
            let [k0, k1, t0, t1] = [0, 1, 2, 3].map(|j| rom.read_byte(b + j));
            DecodedEntry {
                key_world: (k0 & 0x0F) as usize,
                key_pos: ((k0 >> 4) as usize - 2, (k1 & 0x0F) as usize * 16 + (k1 >> 4) as usize),
                away: i * ENTRY_LEN < boundary,
                target: (i * ENTRY_LEN >= boundary).then(|| ((t0 >> 4) as usize - 2, t1 as usize)),
            }
        })
        .collect()
}

/// One decoded table entry.
#[cfg(test)]
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct DecodedEntry {
    pub key_world: usize,
    /// The fortress's grid cell. Every row is distinct — the key is the player's
    /// raw map position, not a folded completion index.
    pub key_pos: (usize, usize),
    /// Whether the target is in another world, so its bit went to the packed
    /// store rather than to the effect.
    pub away: bool,
    /// The target cell, for a home entry. An away entry stores a packed-store
    /// address instead, which does not decode back to a cell.
    pub target: Option<(usize, usize)>,
}

#[cfg(test)]
mod asm_checks {
    use mos6502::cpu::CPU;
    use mos6502::instruction::Ricoh2a03;
    use mos6502::memory::{Bus, Memory};

    use super::*;
    use crate::randomize::completion_bits;
    use crate::randomize::rom_data::{self, FS_PACK_PLANE, FS_UNPACK_PLANE, asm};

    const ROM_PATH: &str = "roms/Super Mario Bros. 3 (USA) (Rev 1).nes";

    fn vanilla() -> Option<Rom> {
        let bytes = std::fs::read(ROM_PATH).ok()?;
        Some(Rom::from_bytes(&bytes).expect("vanilla ROM parses"))
    }

    fn seeds() -> u64 {
        std::env::var("CENSUS_SEEDS").ok().and_then(|s| s.parse().ok()).unwrap_or(10)
    }

    // --- Structure ------------------------------------------------------

    /// The zero-page bytes the effect touches, every one of them an engine
    /// variable rather than scratch this routine invented.
    ///
    /// `$0A`/`$0B` and `$0E`/`$0F` are `Temp_Var11`/`12` and `Temp_Var15`/`16`,
    /// which is exactly what vanilla's own `MO_DoFortressFX` parked its VRAM
    /// address and tile-memory pointer in, across this same window. `$15` is
    /// `Counter_1`, `$20` is `Map_ClearLevelFXCnt`, `$75`/`$77`/`$79` are the
    /// player's map position, `$FD` is `Map_Scroll_X`. Borrowing them is exactly
    /// as safe as the routine this replaces.
    const FX_ENGINE_VARS: [u8; 10] = [0x0A, 0x0B, 0x0E, 0x0F, 0x15, 0x20, 0x75, 0x77, 0x79, 0xFD];

    #[test]
    fn the_effect_is_well_formed() {
        asm::check(&FORTRESS_FX)
            .allocation(FS_FORTRESS_FX)
            // Four internal `JMP`s carry absolute addresses. Relocating the
            // routine without this check would leave them pointing into
            // whatever moved in underneath.
            .origin(FORTRESS_FX_CPU)
            .zero_page(NMI_SAFE_MAX, &FX_ENGINE_VARS)
            .assert_ok();
    }

    /// [`REMOVABLE_PAIRS`] is a Rust copy of eight ROM bytes, and a copy that is
    /// never compared is a copy that drifts. This is the comparison.
    ///
    /// It reads the *vanilla* addresses on purpose:
    /// [`relocate_removable_tables`] leaves them untouched, so an unpatched ROM
    /// and a patched one answer this identically — which is what makes the
    /// relocation auditable rather than merely asserted.
    #[test]
    fn the_relocated_tables_match_vanilla() {
        let Some(rom) = vanilla() else {
            eprintln!("SKIP: requires the ROM");
            return;
        };
        let want: Vec<(u8, u8)> = (0..8)
            .map(|i| {
                (rom.read_byte(MAP_REMOVABLE_VANILLA + i), rom.read_byte(MAP_REMOVE_TO_VANILLA + i))
            })
            .collect();
        assert_eq!(
            &REMOVABLE_PAIRS[..8],
            want,
            "the first eight rows no longer match the ROM's own table"
        );

        // Anything past the eighth is ours, and every one of them has to obey
        // the two rules the doc comment states — otherwise the effect draws the
        // revealed tile in the wrong palette, or reveals something the player
        // cannot walk on.
        //
        // The whole vocabulary, not just the base rows: a numbered lock that
        // crossed a page or revealed a wall would be just as broken, and these
        // are the rows nobody wrote out by hand.
        let vocabulary = obstacle_vocabulary();
        for &(obstacle, revealed) in &vocabulary[8..] {
            // **The water-gap variants are a deliberate exception**, and the
            // only one. Page 2 has no index to spare, so they sit in page 3 and
            // reveal a page-2 bridge: the lock draws in the wrong palette, and
            // so does the bridge until the next map reload. See
            // [`WATER_ORIENTATION`] for why that trade was taken.
            if HINT_TILES[WATER_ORIENTATION].contains(&obstacle) {
                assert_eq!(revealed, rom_data::BRIDGE_TILE, "a water variant reveals a bridge");
                continue;
            }
            assert_eq!(
                obstacle >> 6,
                revealed >> 6,
                "{obstacle:#04X} -> {revealed:#04X} crosses a palette page"
            );
            // Only an obstacle that blocks a *corridor* has to reveal something
            // walkable. A fortress reveals rubble, which is a node the player is
            // already standing on — `Map_CheckDoMove` never tests a destination
            // cell's own byte, so rubble does not belong to either direction
            // list and must not be held to one.
            if rom_data::is_gap_tile(obstacle) {
                assert!(
                    rom_data::VALID_HORZ.contains(&revealed)
                        || rom_data::VALID_VERT.contains(&revealed),
                    "{obstacle:#04X} blocks a corridor but reveals {revealed:#04X}, \
                     which is walkable in no direction"
                );
            }
        }
    }

    /// **The digit is the world the player sees, not the internal index.**
    ///
    /// World order renumbers the worlds and the maze forces it on, so those two
    /// are different in essentially every seed — a lock stamped with the
    /// internal index names a world whose number appears nowhere in the game.
    /// Nothing else catches it: the tile is well-formed, the table has its row,
    /// the lock opens. It is only *wrong*, and only to a player.
    ///
    /// Checked as a multiset, because an away entry stores a packed-store
    /// address rather than a cell and so cannot be matched to its lock
    /// positionally. Every away lock contributes the display number of the world
    /// its fortress is in; every numbered cell contributes the digit it shows.
    /// The two have to agree.
    ///
    /// This also pins the pipeline order it rests on — `world_order::randomize`
    /// writes the table, `lock_keys::apply` reads it — since an unwritten table
    /// silently falls back to the internal index and would look like this test
    /// passing on a vanilla-numbered ROM.
    #[test]
    fn the_digit_is_the_world_the_player_sees() {
        let Ok(bytes) = std::fs::read(ROM_PATH) else {
            eprintln!("SKIP: requires the ROM");
            return;
        };
        let mut shuffled_seeds = 0usize;
        for seed in 0..seeds() {
            let options = crate::Options {
                world_maze: true,
                palettes: false,
                palette_themed: false,
                ..Default::default()
            };
            let Ok((rom, _)) =
                crate::randomize_rom_with_overworld_capture(&bytes, seed, &options, None)
            else {
                continue;
            };

            let display: Vec<usize> = (0..8).map(|w| displayed_world(&rom, w)).collect();
            assert_eq!(
                display.iter().copied().collect::<std::collections::BTreeSet<_>>().len(),
                8,
                "seed {seed}: the display table is not a permutation — world_order either did \
                 not run or ran after this module"
            );
            if display != vec![1, 2, 3, 4, 5, 6, 7, 8] {
                shuffled_seeds += 1;
            }

            let mut want: Vec<usize> = decode_entries(&rom)
                .iter()
                .filter(|e| e.away)
                .map(|e| display[e.key_world])
                .collect();

            let mut got: Vec<usize> = Vec::new();
            for world in 0..8 {
                let info = &rom_data::MAP_TILE_GRIDS[world];
                for screen in 0..info.screens {
                    for row in 0..9 {
                        for col in 0..16 {
                            let off = rom_data::map_tile_offset(world, row, screen * 16 + col);
                            let tile = rom.read_byte(off);
                            if let Some(set) = HINT_TILES.iter().find(|set| set.contains(&tile)) {
                                got.push(set.iter().position(|&t| t == tile).unwrap() + 1);
                            }
                        }
                    }
                }
            }

            want.sort_unstable();
            got.sort_unstable();
            assert_eq!(
                got, want,
                "seed {seed}: the digits on the map are not the display numbers of the \
                 worlds the fortresses are in"
            );
        }
        assert!(
            shuffled_seeds > 0,
            "every sampled seed left the worlds in vanilla order, so this proves nothing about \
             renumbering"
        );
    }

    /// **The row bound holds, and every away lock gets its digit.**
    ///
    /// `removable_rows`' bound is arithmetic — 6 terrain rows plus at most one
    /// per lock, against 17 locks — but both terms are measured properties of
    /// the builder rather than enforced ones, so this walks real builds. The
    /// second half is what catches a silent regression: a numbered tile that
    /// failed to stamp would leave an away lock plain, and the map would simply
    /// stop hinting without anything failing.
    #[test]
    fn the_obstacle_table_never_overflows() {
        let Ok(bytes) = std::fs::read(ROM_PATH) else {
            eprintln!("SKIP: requires the ROM");
            return;
        };
        let mut worst = 0usize;
        for seed in 0..seeds() {
            let options = crate::Options {
                world_maze: true,
                palettes: false,
                palette_themed: false,
                ..Default::default()
            };
            let Ok((rom, _)) =
                crate::randomize_rom_with_overworld_capture(&bytes, seed, &options, None)
            else {
                continue;
            };
            let present = tiles_on_map(&rom);
            let used =
                obstacle_vocabulary().into_iter().filter(|&(t, _)| present[t as usize]).count();
            assert!(used <= REMOVABLE_COUNT, "seed {seed} needs {used} rows");
            worst = worst.max(used);

            // Every away lock should have been numbered. An away entry stores a
            // packed-store address rather than a cell, so the check is by count:
            // numbered cells on the map against away entries in the table.
            let away = decode_entries(&rom).iter().filter(|e| e.away).count();
            let mut numbered = 0usize;
            for world in 0..8 {
                let info = &rom_data::MAP_TILE_GRIDS[world];
                for screen in 0..info.screens {
                    for row in 0..9 {
                        for col in 0..16 {
                            let off = rom_data::map_tile_offset(world, row, screen * 16 + col);
                            if numbered_lock(rom.read_byte(off)).is_some() {
                                numbered += 1;
                            }
                        }
                    }
                }
            }
            // **Absence of a digit has to mean one thing.** Every away lock
            // carries its number, so a lock without one is local — which is
            // what let the older map-object hint go. A stamp that silently
            // failed would make absence ambiguous, and nothing else would say.
            assert_eq!(
                numbered, away,
                "seed {seed}: {away} away locks but {numbered} numbered cells"
            );
        }
        eprintln!("worst row count {worst}/{REMOVABLE_COUNT}");
    }

    /// The helper decodes, fits its allocation, and its self-reference resolves.
    #[test]
    fn the_ml_range_helper_is_well_formed() {
        asm::check(&ML_RANGE)
            .allocation(rom_data::FS_ML_RANGE)
            // `CMP UPPER,X` names a table inside the routine.
            .origin(ML_RANGE_CPU)
            .data_from(11)
            .assert_ok();
    }

    /// The splice replaces one whole instruction with another of the same
    /// length, which is what lets the `BCS` behind it keep both its address and
    /// its operand.
    ///
    /// If vanilla's five bytes were ever not what this expects, the `JSR` would
    /// land mid-instruction and the reload would execute an operand.
    #[test]
    fn the_ml_range_splice_lands_on_an_instruction() {
        let Some(mut rom) = vanilla() else {
            eprintln!("SKIP: requires the ROM");
            return;
        };
        assert_eq!(
            rom.read_range(PRG012_ML_TEST, 5),
            PRG012_ML_TEST_VANILLA,
            "the reload's M/L test is not the `CMP $A400,X` / `BCS` this replaces"
        );

        let rows = removable_rows(&rom);
        relocate_removable_tables(&mut rom, &rows);

        let after = rom.read_range(PRG012_ML_TEST, 5);
        assert_eq!(after[0], 0x20, "JSR");
        assert_eq!(&after[1..3], ML_RANGE_CPU.to_le_bytes(), "JSR names the helper");
        assert_eq!(
            &after[3..],
            &PRG012_ML_TEST_VANILLA[3..],
            "the BCS behind the splice moved, so its operand is now wrong"
        );
        assert_eq!(rom.read_range(rom_data::FS_ML_RANGE, ML_RANGE.len()), ML_RANGE);
    }

    /// **The bounds are one past the last tile any vanilla map places.**
    ///
    /// That is the whole safety argument for narrowing the M/L test: no tile the
    /// game actually uses falls outside its page's window, so no cell changes
    /// behavior. Checked against the eight grids rather than asserted, because
    /// the claim is about the ROM and not about our intent.
    #[test]
    fn the_ml_bounds_exclude_nothing_the_maps_use() {
        let Some(rom) = vanilla() else {
            eprintln!("SKIP: requires the ROM");
            return;
        };
        for world in 0..8 {
            let info = &rom_data::MAP_TILE_GRIDS[world];
            for screen in 0..info.screens {
                for row in 0..9 {
                    for col in 0..16 {
                        let tile =
                            rom.read_byte(rom_data::map_tile_offset(world, row, screen * 16 + col));
                        let page = (tile >> 6) as usize;
                        assert!(
                            tile < ML_RANGE_UPPER[page],
                            "W{} places {tile:#04X}, at or above its page bound {:#04X} — \
                             narrowing the M/L test would change how that cell completes",
                            world + 1,
                            ML_RANGE_UPPER[page]
                        );
                    }
                }
            }
        }
    }

    /// The relocation's three writes, checked where they land rather than where
    /// they were aimed: the copy is byte-identical to vanilla's table, both
    /// operands name the copy, and the scan count matches the entry count.
    ///
    /// The operand check is the one that matters. A relocated table that
    /// nothing points at is not a bug the ROM reports — the scan would run over
    /// whatever still sits at `$A437` and keep working, right up until an entry
    /// is added and only half the game sees it.
    #[test]
    fn the_relocation_repoints_both_readers() {
        let Some(mut rom) = vanilla() else {
            eprintln!("SKIP: requires the ROM");
            return;
        };
        let rows = removable_rows(&rom);
        relocate_removable_tables(&mut rom, &rows);

        for (i, &(from, to)) in rows.iter().enumerate() {
            assert_eq!(rom.read_byte(MAP_REMOVABLE_TILES + i), from, "removable[{i}]");
            assert_eq!(rom.read_byte(MAP_REMOVE_TO_TILES + i), to, "remove_to[{i}]");
        }
        assert_eq!(
            rom.read_range(PRG012_REMOVABLE_OPERAND, 2),
            prg_bank_file_to_cpu(12, MAP_REMOVABLE_TILES).to_le_bytes(),
            "`CMP Map_Removable_Tiles,X` still names the vanilla table"
        );
        assert_eq!(
            rom.read_range(PRG012_REMOVE_TO_OPERAND, 2),
            prg_bank_file_to_cpu(12, MAP_REMOVE_TO_TILES).to_le_bytes(),
            "`LDA Map_RemoveTo_Tiles,X` still names the vanilla table"
        );
        assert_eq!(
            rom.read_byte(PRG012_SCAN_COUNT),
            (REMOVABLE_COUNT - 1) as u8,
            "the scan count and the table length disagree"
        );

        // The opcodes either side of the operands: proof the patch landed on
        // whole instructions and not mid-stream.
        assert_eq!(rom.read_byte(PRG012_REMOVABLE_OPERAND - 1), 0xDD, "CMP abs,X");
        assert_eq!(rom.read_byte(PRG012_REMOVE_TO_OPERAND - 1), 0xBD, "LDA abs,X");
        assert_eq!(rom.read_byte(PRG012_SCAN_COUNT - 1), 0xA2, "LDX #");
    }

    /// **The one word in the ROM that names the routine.**
    ///
    /// Repointing it is what frees vanilla's 236 bytes of slot tables and its
    /// 301-byte routine. If the jump table moved, this repoint would overwrite
    /// an unrelated map operation and the effect would never run.
    #[test]
    fn the_jump_table_entry_is_where_the_repoint_expects_it() {
        let Some(rom) = vanilla() else {
            eprintln!("SKIP: requires the ROM");
            return;
        };
        assert_eq!(
            rom.read_range(MAP_OP8_VECTOR, 2),
            MAP_OP8_VANILLA.to_le_bytes(),
            "map operation 8 is not MO_DoFortressFX"
        );
        // Operation 7 is MO_DoLevelClear, in PRG011 — the routine whose two
        // branches (map cell and map object) both end at PRG011_ABBE setting
        // Map_Operation = 8. That shared exit is why a World 8 tank reaches
        // this effect exactly as a stone fortress does.
        assert_eq!(
            rom.read_range(MAP_OP8_VECTOR - 2, 2),
            0xA9EBu16.to_le_bytes(),
            "operation 7 is not MO_DoLevelClear"
        );

        let mut patched = rom.clone();
        apply(&mut patched, &[]);
        assert_eq!(patched.read_range(MAP_OP8_VECTOR, 2), FORTRESS_FX_CPU.to_le_bytes());
    }

    /// The six table reads name the entry table, in order, and the two patched
    /// immediates are the instructions this module thinks they are. Get any of
    /// them wrong and the scan reads someone else's bytes, which decode and run
    /// perfectly well.
    #[test]
    fn the_routine_reads_its_own_table() {
        let operand = |at: usize, opcode: u8| {
            assert_eq!(FORTRESS_FX[at], opcode, "offset {at} is not the expected indexed load");
            u16::from_le_bytes([FORTRESS_FX[at + 1], FORTRESS_FX[at + 2]])
        };
        for (at, opcode, want, name) in [
            (37, 0xBD, ENTRIES_CPU, "key0"),
            (44, 0xBD, ENTRIES_CPU + 1, "key1"),
            (74, 0xBC, ENTRIES_CPU + 2, "away plane offset"),
            (77, 0xBD, ENTRIES_CPU + 3, "away bit mask"),
            (96, 0xBD, ENTRIES_CPU + 2, "home row byte"),
            (102, 0xBD, ENTRIES_CPU + 3, "home column"),
        ] {
            assert_eq!(operand(at, opcode), want, "offset {at} should read {name}");
        }
        assert_eq!(FORTRESS_FX[COUNT_OPERAND - 1], 0xA2, "COUNT_OPERAND is not an LDX immediate");
        assert_eq!(
            FORTRESS_FX[BOUNDARY_OPERAND - 1],
            0xE0,
            "BOUNDARY_OPERAND is not a CPX immediate"
        );
    }

    /// The reservations this module sizes itself against are the registry's.
    #[test]
    fn the_reservations_match_the_registry() {
        let row = |off: usize| {
            rom_data::FREE_SPACE_ALLOCATIONS
                .iter()
                .find(|a| a.offset == off)
                .unwrap_or_else(|| panic!("{off:#07X} needs a FREE_SPACE_ALLOCATIONS row"))
        };
        assert!(FORTRESS_FX.len() <= row(FS_FORTRESS_FX).size);
        assert_eq!(row(FS_LOCK_MIRROR).size, MIRROR_LEN);
        assert_eq!(row(FS_LOCK_ENTRIES).size, ENTRIES_RESERVED);
    }

    /// `PACKED` is private to [`completion_bits`], so this module restates it.
    /// Read it back out of the bytes that module writes to the ROM: if the store
    /// ever moves, the two disagree here rather than on a cartridge.
    #[test]
    fn the_packed_base_matches_completion_bits() {
        let Some(rom) = vanilla() else {
            eprintln!("SKIP: requires the ROM");
            return;
        };
        let mut patched = rom.clone();
        completion_bits::apply(&mut patched);

        let sta = [0x9D, PACKED as u8, (PACKED >> 8) as u8]; // STA PACKED,X
        let lda = [0xBD, PACKED as u8, (PACKED >> 8) as u8]; // LDA PACKED,X
        assert!(
            patched.read_range(FS_PACK_PLANE, 99).windows(3).any(|w| w == sta),
            "PACK_PLANE does not store to ${PACKED:04X} — the packed store moved"
        );
        assert!(
            patched.read_range(FS_UNPACK_PLANE, 72).windows(3).any(|w| w == lda),
            "UNPACK_PLANE does not read ${PACKED:04X} — the packed store moved"
        );
    }

    /// The mirror is PRG012's own tables, not a hand copy of them.
    #[test]
    fn the_mirror_is_the_engines_own_answer() {
        let Some(mut rom) = vanilla() else {
            eprintln!("SKIP: requires the ROM");
            return;
        };
        // The mirror is built from the tables at their randomized address, so
        // this has to stand where `apply` stands: after the relocation.
        let rows = removable_rows(&rom);
        relocate_removable_tables(&mut rom, &rows);
        let mirror = mirror_bytes(&rom);
        assert_mirror_agrees_with_rust(&mirror);

        // The mirror is a copy of the table, so it must carry exactly the rows
        // this map earned — no more, in the same order.
        // `the_relocated_tables_match_vanilla` is what pins the base rows to the
        // ROM; this only has to prove the copy is faithful and complete.
        let (obstacles, revealed): (Vec<u8>, Vec<u8>) = rows.iter().copied().unzip();
        assert_eq!(&mirror[..REMOVABLE_COUNT], obstacles, "the mirror's obstacle half is wrong");
        assert_eq!(
            &mirror[REMOVABLE_COUNT..2 * REMOVABLE_COUNT],
            revealed,
            "the mirror's remove-to half is wrong"
        );
        for i in 0..REMOVABLE_COUNT {
            let to = mirror[REMOVABLE_COUNT + i] as usize;
            let want: [u8; 4] =
                PATTERN_QUADRANT_ORDER.map(|q| rom.read_byte(PRG012_FILE_BASE + q * 256 + to));
            let base = 2 * REMOVABLE_COUNT + i * 4;
            assert_eq!(
                &mirror[base..base + 4],
                &want,
                "entry {i} does not carry the quadrants of {to:#04X}"
            );
        }

        let mut patched = rom.clone();
        apply(&mut patched, &[]);
        assert_eq!(patched.read_range(FS_LOCK_MIRROR, MIRROR_LEN), mirror);
    }

    /// **The orb's arming is vanilla's and this module leaves it alone.**
    ///
    /// All 17 Boom-Booms carry a non-zero Y-nibble already, and every fortress
    /// keys exactly one lock, so there is nothing to arm and nothing to clear.
    /// An earlier cut zeroed these bytes, which would now disarm the effect
    /// entirely.
    #[test]
    fn the_enemy_data_is_not_touched() {
        let Some(rom) = vanilla() else {
            eprintln!("SKIP: requires the ROM");
            return;
        };
        for &off in &rom_data::BOOMBOOM_Y_OFFSETS {
            assert_ne!(
                rom.read_byte(off) & 0xF0,
                0,
                "Boom-Boom at {off:#07X} is unarmed in vanilla, so the orb would never fire"
            );
        }
        let mut patched = rom.clone();
        apply(&mut patched, &[]);
        for &off in &rom_data::BOOMBOOM_Y_OFFSETS {
            assert_eq!(patched.read_byte(off), rom.read_byte(off), "{off:#07X} was written");
        }
    }

    #[test]
    #[should_panic(expected = "opens both")]
    fn one_fortress_cannot_open_two_locks() {
        assert_one_key_per_lock(&[
            LockEntry { key_world: 1, key_pos: (2, 3), target_world: 1, target_pos: (4, 5) },
            LockEntry { key_world: 1, key_pos: (2, 3), target_world: 4, target_pos: (6, 7) },
        ]);
    }

    #[test]
    #[should_panic(expected = "is opened by both")]
    fn one_lock_cannot_have_two_fortresses() {
        assert_one_key_per_lock(&[
            LockEntry { key_world: 1, key_pos: (2, 3), target_world: 1, target_pos: (4, 5) },
            LockEntry { key_world: 1, key_pos: (6, 7), target_world: 1, target_pos: (4, 5) },
        ]);
    }

    // --- Executing it ---------------------------------------------------
    //
    // `asm::check` proves the bytes decode; it cannot see that they compute the
    // right answer. The effect reads live engine state and calls three engine
    // routines, but almost all of it can still be run — and the half worth
    // running is exactly the half this rework replaced. Vanilla read a VRAM
    // address, a completion column and bit, four CHR quadrants and a replacement
    // tile out of seven parallel tables; this derives all five from the entry it
    // finds by position. Nothing else checks that arithmetic.
    //
    // The run stops at `WorldMap_UpdateAndDraw`, which is where every exit path
    // jumps and which would otherwise draw a whole frame.

    /// `MO_DoFortressFX`'s exit, and the harness's stopping point.
    const WORLDMAP_UPDATE_DRAW: u16 = 0xCF29;
    /// `Tile_Mem` — four screens of map RAM, `$1B0` apart, grid row 0 at `+$110`.
    const TILE_MEM: u16 = 0x6000;
    const SCREEN_STRIDE: u16 = 0x1B0;
    const MAP_ROW0: u16 = 0x110;
    const GRAPHICS_BUFCNT: u16 = 0x0300;
    const GRAPHICS_BUFFER: u16 = 0x0301;
    const CLEAR_FX_CNT: u16 = 0x0020;
    const INTRO_TICK: u16 = 0x0711;
    const COUNTER_1: u16 = 0x0015;
    const SCROLL_X: u16 = 0x00FD;
    const MAP_DO_FX: u16 = 0x0745;
    /// The two bytes the matched entry's target is parked in.
    const FX_ROW: u16 = 0x0743;
    const FX_COL: u16 = 0x0744;

    fn cell_addr(row: usize, col: usize) -> u16 {
        TILE_MEM
            + (col as u16 / 16) * SCREEN_STRIDE
            + MAP_ROW0
            + row as u16 * 0x10
            + (col as u16 % 16)
    }

    /// A CPU mid-map with the effect armed and `player` standing on the cell at
    /// `at` in `world` — which is what the `(?)` orb leaves behind.
    ///
    /// PRG030 joins the two map banks because `Tile_Mem_Addr`, the per-screen
    /// base table the effect indexes, lives at `$8000`.
    fn armed_at(rom: &Rom, world: usize, at: (usize, usize), player: u8) -> CPU<Memory, Ricoh2a03> {
        let mut mem = Memory::new();
        mem.set_bytes(0x8000, rom.read_range(0x3C010, 0x2000));
        mem.set_bytes(0xA000, rom.read_range(0x16010, 0x2000));
        mem.set_bytes(0xC000, rom.read_range(0x14010, 0x2000));

        let grid = rom_data::read_tile_grid(rom, world);
        for row in 0..grid.rows() {
            for col in 0..grid.cols {
                mem.set_byte(cell_addr(row, col), grid.get(row, col));
            }
        }

        mem.set_byte(PLAYER_CURRENT, player);
        mem.set_byte(WORLD_NUM, world as u8);
        // The player's live position, sub-tile offsets and all — the effect
        // masks them off itself, and a fixture that pre-masked them would not
        // be testing that.
        let (row, col) = at;
        mem.set_byte(0x0075 + player as u16, (((row + 2) as u8) << 4) | 0x08);
        mem.set_byte(0x0077 + player as u16, (col / 16) as u8);
        mem.set_byte(0x0079 + player as u16, (((col % 16) as u8) << 4) | 0x04);

        mem.set_byte(MAP_DO_FX, 1); // the orb
        // One tick left, so the mono-flash finishes on this very frame.
        mem.set_byte(INTRO_TICK, 1);
        mem.set_byte(CLEAR_FX_CNT, 0);
        // Not a multiple of four, so the poof counter is left alone and the
        // `$20` the visibility check wrote is what this reads back.
        mem.set_byte(COUNTER_1, 1);
        mem.set_byte(GRAPHICS_BUFCNT, 0);
        mem.set_byte(SCROLL_X, 0);

        CPU::new(mem, Ricoh2a03)
    }

    /// Run the effect from the map-operation jump table's own vector.
    fn run_effect(cpu: &mut CPU<Memory, Ricoh2a03>) {
        cpu.registers.stack_pointer = mos6502::registers::StackPointer(0xFD);
        cpu.registers.program_counter = FORTRESS_FX_CPU;
        for _ in 0..200_000 {
            if cpu.registers.program_counter == WORLDMAP_UPDATE_DRAW {
                return;
            }
            cpu.single_step();
        }
        panic!("MO_DoFortressFX ran away");
    }

    /// Every byte of the packed store the run touched.
    fn packed(cpu: &mut CPU<Memory, Ricoh2a03>) -> Vec<(u16, u8)> {
        (PACKED..PACKED + 2 * PLANE_RESERVE as u16)
            .filter_map(|a| {
                let b = cpu.memory.get_byte(a);
                (b != 0).then_some((a, b))
            })
            .collect()
    }

    /// Stamp lock tiles at every target cell so the destination owns a
    /// completion bit, then install the packed store exactly as the pipeline
    /// does — every grid write first, `completion_bits` last.
    fn with_locks(rom: &Rom, entries: &[LockEntry]) -> Rom {
        let mut out = rom.clone();
        for e in entries {
            out.write_byte(
                rom_data::map_tile_offset(e.target_world, e.target_pos.0, e.target_pos.1),
                rom_data::LOCK_TILES[0],
            );
        }
        completion_bits::apply(&mut out);
        out
    }

    /// A ROM with one lock installed, plus the away payload it emitted.
    fn one_lock(rom: &Rom, e: LockEntry) -> (Rom, Option<[u8; 2]>) {
        let base = with_locks(rom, &[e]);
        let mut patched = base.clone();
        apply(&mut patched, &[e]);
        let payload = e.is_away().then(|| {
            let map = CompletionMap::from_rom(&base);
            away_target(&map, e.target_world, e.target_pos).expect("the cell owns a bit")
        });
        (patched, payload)
    }

    fn both_planes(payload: [u8; 2]) -> Vec<(u16, u8)> {
        let [offset, mask] = payload;
        vec![(PACKED + offset as u16, mask), (PACKED_MIRROR + offset as u16, mask)]
    }

    /// **The whole derivation, executed.**
    ///
    /// One fortress cleared, one frame: the effect finds its entry by the
    /// player's position, the tile in map RAM becomes the path the engine's own
    /// table names, both players' completion bits go up, and the graphics buffer
    /// carries the VRAM address and the four CHR quadrants that seven deleted
    /// tables used to hold.
    #[test]
    fn the_effect_derives_everything_the_slot_tables_used_to_store() {
        let Some(rom) = vanilla() else {
            eprintln!("SKIP: requires the ROM");
            return;
        };
        // Fortress and lock on the same half-screen, so the lock is on screen
        // and the full animation runs; columns 4 and 5 sit inside the edge
        // filter's `[$10, $E8)` window at scroll 0.
        let (world, fort, lock_at) = (1usize, (2usize, 4usize), (3usize, 5usize));
        let e =
            LockEntry { key_world: world, key_pos: fort, target_world: world, target_pos: lock_at };
        let (patched, _) = one_lock(&rom, e);

        let mirror = mirror_bytes(&patched);
        let idx = (0..REMOVABLE_COUNT)
            .find(|&i| mirror[i] == rom_data::LOCK_TILES[0])
            .expect("the lock tile is removable");
        let path = mirror[REMOVABLE_COUNT + idx];

        let mut cpu = armed_at(&patched, world, fort, 0);
        assert_eq!(cpu.memory.get_byte(cell_addr(lock_at.0, lock_at.1)), rom_data::LOCK_TILES[0]);
        run_effect(&mut cpu);

        // The entry was found by position and parked for the poof frames.
        assert_eq!(cpu.memory.get_byte(FX_ROW), home_target(lock_at)[0]);
        assert_eq!(cpu.memory.get_byte(FX_COL), home_target(lock_at)[1]);

        // The map RAM write.
        assert_eq!(
            cpu.memory.get_byte(cell_addr(lock_at.0, lock_at.1)),
            path,
            "the cell did not become the path Map_RemoveTo_Tiles names"
        );

        // The completion bits, both players.
        let bit = MAP_COMPLETE_BITS[lock_at.0.min(7)];
        assert_eq!(cpu.memory.get_byte(MAP_COMPLETIONS + lock_at.1 as u16), bit, "Mario");
        assert_eq!(cpu.memory.get_byte(MAP_COMPLETIONS + 0x40 + lock_at.1 as u16), bit, "Luigi");

        // The graphics buffer: two runs of two patterns, 32 bytes apart, then a
        // terminator. `$2880 + row*64 + col_in_screen*2` — no screen term,
        // because every screen aliases onto the same nametable window.
        let vram = 0x2880u16 + lock_at.0 as u16 * 64 + (lock_at.1 % 16) as u16 * 2;
        let q = &mirror[2 * REMOVABLE_COUNT + idx * 4..2 * REMOVABLE_COUNT + idx * 4 + 4];
        let want = [
            (vram >> 8) as u8,
            vram as u8,
            0x02,
            q[0],
            q[1],
            ((vram + 32) >> 8) as u8,
            (vram + 32) as u8,
            0x02,
            q[2],
            q[3],
            0x00,
        ];
        let got: Vec<u8> =
            (0..want.len() as u16).map(|i| cpu.memory.get_byte(GRAPHICS_BUFFER + i)).collect();
        assert_eq!(got, want, "the queued VRAM write is not the target cell's");
        assert_eq!(cpu.memory.get_byte(GRAPHICS_BUFCNT), 10, "buffer count past the terminator");
        assert_eq!(cpu.memory.get_byte(CLEAR_FX_CNT), 1, "$20 = 1 is the full-animate path");
    }

    /// **The regression this whole design exists to avoid: a fortress the
    /// player stands on that is not a fortress tile.**
    ///
    /// World 8's tanks and battleships are map-object sprites floating over a
    /// cell the overworld writer deliberately blanks to a path node — and three
    /// of World 8's four fortresses get one. An earlier cut of this rework keyed
    /// on `Map_MarkLevelComplete`'s fortress branch, which is gated on the tile
    /// under the player being rubble, and **17.6% of all locks in every seed
    /// were silently dead**. Reading the player's position at map operation 8
    /// has no such gate, because both halves of `MO_DoLevelClear` — the map-cell
    /// clear and the map-object poof — set `Map_Operation = 8` at the same
    /// shared exit.
    #[test]
    fn a_fortress_under_a_sprite_still_opens_its_lock() {
        let Some(rom) = vanilla() else {
            eprintln!("SKIP: requires the ROM");
            return;
        };
        let (world, fort, lock_at) = (1usize, (2usize, 4usize), (3usize, 5usize));
        let e =
            LockEntry { key_world: world, key_pos: fort, target_world: world, target_pos: lock_at };
        let (patched, _) = one_lock(&rom, e);

        let mut cpu = armed_at(&patched, world, fort, 0);
        // Blank the key cell exactly as the writer does under an army sprite:
        // a plain path node, nothing a rubble test would ever accept.
        cpu.memory.set_byte(cell_addr(fort.0, fort.1), 0x45);
        run_effect(&mut cpu);

        assert_ne!(
            cpu.memory.get_byte(cell_addr(lock_at.0, lock_at.1)),
            rom_data::LOCK_TILES[0],
            "the lock stayed shut — the key is gated on the tile under the player again"
        );
        assert_eq!(
            cpu.memory.get_byte(MAP_COMPLETIONS + lock_at.1 as u16),
            MAP_COMPLETE_BITS[lock_at.0.min(7)]
        );
    }

    /// **An away lock: a fortress in one world sets a lock's bit in another.**
    ///
    /// Both planes, exactly one bit each, nothing else in the store — and no
    /// flash, no poof and no graphics buffer, because there is nothing on this
    /// screen to animate and the entry carries a store address rather than a
    /// cell.
    #[test]
    fn an_away_fortress_sets_exactly_one_bit_in_each_plane() {
        let Some(rom) = vanilla() else {
            eprintln!("SKIP: requires the ROM");
            return;
        };
        let e = LockEntry { key_world: 0, key_pos: (4, 5), target_world: 3, target_pos: (2, 9) };
        let (patched, payload) = one_lock(&rom, e);

        let mut cpu = armed_at(&patched, 0, e.key_pos, 0);
        run_effect(&mut cpu);

        assert_eq!(packed(&mut cpu), both_planes(payload.unwrap()), "both planes, one bit each");
        assert_eq!(cpu.memory.get_byte(GRAPHICS_BUFCNT), 0, "nothing was queued");
        assert_eq!(cpu.memory.get_byte(MAP_DO_FX), 0, "the effect disarmed itself");
        assert_eq!(cpu.memory.get_byte(INTRO_TICK), 1, "the flash never ran");
    }

    /// Either player's clear opens the same lock: the key is read through
    /// `Player_Current`, not from Mario's bytes.
    #[test]
    fn either_player_opens_the_same_lock() {
        let Some(rom) = vanilla() else {
            eprintln!("SKIP: requires the ROM");
            return;
        };
        let e = LockEntry { key_world: 6, key_pos: (6, 33), target_world: 1, target_pos: (3, 12) };
        let (patched, payload) = one_lock(&rom, e);

        for player in 0..2u8 {
            let mut cpu = armed_at(&patched, 6, e.key_pos, player);
            run_effect(&mut cpu);
            assert_eq!(packed(&mut cpu), both_planes(payload.unwrap()), "player {player}");
        }
    }

    /// **The key is the player's own cell, every row and every screen.**
    ///
    /// Grid rows 7 and 8 are the ones worth having: `Map_MarkLevelComplete`'s
    /// completion index folds them onto one value, and reading `World_Map_Y`
    /// directly does not — so two fortresses one row apart on the bottom of the
    /// map no longer share a key.
    #[test]
    fn the_key_is_the_players_own_cell() {
        let Some(rom) = vanilla() else {
            eprintln!("SKIP: requires the ROM");
            return;
        };
        for row in 0..9usize {
            for col in [0usize, 5, 16, 47] {
                let e = LockEntry {
                    key_world: 4,
                    key_pos: (row, col),
                    target_world: 0,
                    target_pos: (2, 4),
                };
                let (patched, payload) = one_lock(&rom, e);
                let mut cpu = armed_at(&patched, 4, (row, col), 0);
                run_effect(&mut cpu);
                assert_eq!(
                    packed(&mut cpu),
                    both_planes(payload.unwrap()),
                    "row {row} column {col} did not match its own key"
                );
            }
        }
    }

    /// Standing anywhere else opens nothing — and, since every fortress arms the
    /// orb but only its own entry matches, "anywhere else" includes every
    /// fortress that opens a different lock.
    #[test]
    fn a_non_matching_position_opens_nothing() {
        let Some(rom) = vanilla() else {
            eprintln!("SKIP: requires the ROM");
            return;
        };
        let e = LockEntry { key_world: 2, key_pos: (4, 5), target_world: 7, target_pos: (6, 30) };
        let (patched, _) = one_lock(&rom, e);

        for (world, at, why) in [
            (3usize, (4usize, 5usize), "another world"),
            (2, (5, 5), "another row"),
            (2, (4, 6), "another column"),
            (2, (4, 21), "the same column on another screen"),
        ] {
            let mut cpu = armed_at(&patched, world, at, 0);
            run_effect(&mut cpu);
            assert!(packed(&mut cpu).is_empty(), "{why} still set a far bit");
            assert_eq!(cpu.memory.get_byte(GRAPHICS_BUFCNT), 0, "{why} still queued a write");
            assert_eq!(cpu.memory.get_byte(MAP_DO_FX), 0, "{why} left the effect armed");
        }
    }

    /// A lock the player cannot see updates map RAM and the completion bits but
    /// queues no VRAM write.
    ///
    /// This is the check that keeps the map from being corrupted rather than
    /// merely under-animated: the VRAM address has **no screen term**, so a lock
    /// sixteen columns away computes an address currently occupied by a visible
    /// tile from a different screen.
    #[test]
    fn an_off_screen_lock_updates_the_map_but_writes_no_tile() {
        let Some(rom) = vanilla() else {
            eprintln!("SKIP: requires the ROM");
            return;
        };
        // The fortress is two screens from its lock, which is the normal shape
        // for a lock the player opens from a distance.
        let (world, fort, lock_at) = (1usize, (2usize, 36usize), (3usize, 5usize));
        let e =
            LockEntry { key_world: world, key_pos: fort, target_world: world, target_pos: lock_at };
        let (patched, _) = one_lock(&rom, e);

        let mut cpu = armed_at(&patched, world, fort, 0);
        run_effect(&mut cpu);

        assert_ne!(
            cpu.memory.get_byte(cell_addr(lock_at.0, lock_at.1)),
            rom_data::LOCK_TILES[0],
            "map RAM is updated whether or not anyone is looking"
        );
        assert_eq!(
            cpu.memory.get_byte(MAP_COMPLETIONS + lock_at.1 as u16),
            MAP_COMPLETE_BITS[lock_at.0]
        );
        assert_eq!(cpu.memory.get_byte(GRAPHICS_BUFCNT), 0, "nothing was queued");
        assert_eq!(cpu.memory.get_byte(CLEAR_FX_CNT), 6, "$20 = 6 is the data-only exit");
    }

    /// A lock whose bit is already set is skipped outright. This is what stops
    /// map rows 7 and 8, which share bit `$01` (#212), from opening each other's
    /// cells.
    #[test]
    fn an_already_busted_lock_is_skipped() {
        let Some(rom) = vanilla() else {
            eprintln!("SKIP: requires the ROM");
            return;
        };
        let (world, fort, lock_at) = (1usize, (2usize, 4usize), (3usize, 5usize));
        let e =
            LockEntry { key_world: world, key_pos: fort, target_world: world, target_pos: lock_at };
        let (patched, _) = one_lock(&rom, e);

        let mut cpu = armed_at(&patched, world, fort, 0);
        cpu.memory.set_byte(MAP_COMPLETIONS + lock_at.1 as u16, MAP_COMPLETE_BITS[lock_at.0]);
        run_effect(&mut cpu);

        assert_eq!(
            cpu.memory.get_byte(cell_addr(lock_at.0, lock_at.1)),
            rom_data::LOCK_TILES[0],
            "the cell was opened twice"
        );
        assert_eq!(cpu.memory.get_byte(GRAPHICS_BUFCNT), 0, "nothing was queued");
        assert_eq!(cpu.memory.get_byte(MAP_DO_FX), 0, "the effect disarmed itself");
    }

    /// A cell holding something `Map_Removable_Tiles` cannot name is not a gap —
    /// the usual reason being that a hammer already opened it — and the effect
    /// stops rather than writing a tile derived from a garbage index.
    #[test]
    fn a_cell_that_is_not_a_gap_stops_the_effect() {
        let Some(rom) = vanilla() else {
            eprintln!("SKIP: requires the ROM");
            return;
        };
        let (world, fort, lock_at) = (1usize, (2usize, 4usize), (3usize, 5usize));
        let e =
            LockEntry { key_world: world, key_pos: fort, target_world: world, target_pos: lock_at };
        let (patched, _) = one_lock(&rom, e);

        let mut cpu = armed_at(&patched, world, fort, 0);
        cpu.memory.set_byte(cell_addr(lock_at.0, lock_at.1), 0x45); // a plain path
        run_effect(&mut cpu);

        assert_eq!(cpu.memory.get_byte(cell_addr(lock_at.0, lock_at.1)), 0x45, "the cell moved");
        assert_eq!(cpu.memory.get_byte(MAP_COMPLETIONS + lock_at.1 as u16), 0, "a bit was set");
        assert_eq!(cpu.memory.get_byte(GRAPHICS_BUFCNT), 0, "nothing was queued");
        assert_eq!(cpu.memory.get_byte(MAP_DO_FX), 0, "the effect disarmed itself");
    }

    /// **A full table, mixed home and away, and the right entry wins.**
    ///
    /// This is the shape the rework exists to enable and that its own output
    /// does not yet exercise: [`MAX_ENTRIES`] locks where vanilla's slot tables
    /// held seventeen, and eight keyed on one world where the four-wide
    /// `FortressFX_W1..W8` row held four.
    #[test]
    fn a_full_mixed_table_opens_the_entry_it_is_standing_on() {
        let Some(rom) = vanilla() else {
            eprintln!("SKIP: requires the ROM");
            return;
        };
        let entries: Vec<LockEntry> = (0..MAX_ENTRIES)
            .map(|n| {
                let key_world = if n < 8 { 2 } else { n % 8 };
                let home = n % 3 == 0 || n < 8;
                LockEntry {
                    key_world,
                    key_pos: (n % 7, 3 + n),
                    target_world: if home { key_world } else { (key_world + 3) % 8 },
                    target_pos: (1 + n % 5, 2 + n),
                }
            })
            .collect();
        assert!(
            entries.iter().filter(|e| e.key_world == 2).count() >= 8,
            "at least eight in one world, twice what the retired per-world row held"
        );
        assert!(entries.iter().any(|e| e.is_away()), "and a mix of both kinds");

        let base = with_locks(&rom, &entries);
        let mut patched = base.clone();
        apply(&mut patched, &entries);
        assert_eq!(decode_entries(&patched).len(), MAX_ENTRIES, "the table must be full");
        let map = CompletionMap::from_rom(&base);

        for (n, e) in entries.iter().enumerate() {
            let mut cpu = armed_at(&patched, e.key_world, e.key_pos, 0);
            run_effect(&mut cpu);
            if e.is_away() {
                let payload = away_target(&map, e.target_world, e.target_pos).unwrap();
                assert_eq!(packed(&mut cpu), both_planes(payload), "entry {n} (away)");
            } else {
                let want = home_target(e.target_pos);
                assert_eq!(
                    [cpu.memory.get_byte(FX_ROW), cpu.memory.get_byte(FX_COL)],
                    want,
                    "entry {n} (home)"
                );
            }
        }
    }

    // --- Mutations ------------------------------------------------------

    /// Run one away lock over a ROM some `mutate` has damaged, and report what
    /// the store ended up holding.
    fn mutated(rom: &Rom, mutate: impl FnOnce(&mut Rom)) -> (Vec<(u16, u8)>, [u8; 2]) {
        let e = LockEntry { key_world: 0, key_pos: (4, 5), target_world: 3, target_pos: (2, 9) };
        let (mut patched, payload) = one_lock(rom, e);
        mutate(&mut patched);
        let mut cpu = armed_at(&patched, 0, e.key_pos, 0);
        run_effect(&mut cpu);
        (packed(&mut cpu), payload.unwrap())
    }

    #[test]
    fn a_wrong_bit_mask_is_caught() {
        let Some(rom) = vanilla() else {
            eprintln!("SKIP: requires the ROM");
            return;
        };
        let (got, payload) = mutated(&rom, |r| {
            let at = FS_LOCK_ENTRIES + 3;
            let mask = r.read_byte(at);
            r.write_byte(at, if mask == 0x80 { 0x40 } else { mask << 1 });
        });
        assert!(!got.is_empty(), "the mutated routine wrote nothing at all");
        assert_ne!(got, both_planes(payload), "a planted wrong bit mask survived");
    }

    #[test]
    fn a_wrong_byte_offset_is_caught() {
        let Some(rom) = vanilla() else {
            eprintln!("SKIP: requires the ROM");
            return;
        };
        let (got, payload) = mutated(&rom, |r| {
            let at = FS_LOCK_ENTRIES + 2;
            let off = r.read_byte(at);
            r.write_byte(at, off + 1);
        });
        assert!(!got.is_empty(), "the mutated routine wrote nothing at all");
        assert_ne!(got, both_planes(payload), "a planted wrong plane offset survived");
    }

    /// Point the mirror write at Mario's plane, which is what "I forgot the
    /// other half" looks like in bytes. The lock would come back after a game
    /// over, because `PRG030_9314` ANDs the two halves.
    #[test]
    fn dropping_the_mirror_write_is_caught() {
        let Some(rom) = vanilla() else {
            eprintln!("SKIP: requires the ROM");
            return;
        };
        let (got, payload) = mutated(&rom, |r| {
            // The `STA PACKED_MIRROR,Y` at routine offset 91.
            r.write_range(FS_FORTRESS_FX + 91, &[0x99, PACKED as u8, (PACKED >> 8) as u8]);
        });
        assert_eq!(
            got,
            both_planes(payload)[..1].to_vec(),
            "the mirror plane was never really written"
        );
    }

    /// Move the boundary and an away lock takes the home branch: it would park a
    /// plane offset and a bit mask in `$0743`/`$0744` and animate a cell that
    /// has nothing to do with the lock.
    #[test]
    fn the_home_away_boundary_is_load_bearing() {
        let Some(rom) = vanilla() else {
            eprintln!("SKIP: requires the ROM");
            return;
        };
        let e = LockEntry { key_world: 0, key_pos: (4, 5), target_world: 3, target_pos: (2, 9) };
        let (mut patched, _) = one_lock(&rom, e);
        assert_eq!(
            patched.read_byte(FS_FORTRESS_FX + BOUNDARY_OPERAND),
            ENTRY_LEN as u8,
            "one away entry"
        );
        patched.write_byte(FS_FORTRESS_FX + BOUNDARY_OPERAND, 0);

        let mut cpu = armed_at(&patched, 0, e.key_pos, 0);
        run_effect(&mut cpu);
        assert!(packed(&mut cpu).is_empty(), "the away branch was never really selected");
    }

    /// **The away payload, checked by expanding it rather than by recomputing
    /// it.**
    ///
    /// Set exactly the bit an away entry names in an otherwise empty store, run
    /// [`CompletionMap::unpack`] — the *other* direction, and the one the console
    /// runs on arrival — and the resulting `Map_Completions` half must carry the
    /// lock cell's bit and nothing else. Over real builds, on every lock every
    /// seed actually placed.
    ///
    /// ```sh
    /// CENSUS_SEEDS=200 cargo test --release --lib the_away_payload_expands_to_the_lock_cell
    /// ```
    #[test]
    fn the_away_payload_expands_to_the_lock_cell() {
        let Ok(rom_bytes) = std::fs::read(ROM_PATH) else {
            eprintln!("SKIP: requires the ROM");
            return;
        };
        let mut checked = 0usize;
        for seed in 0..seeds() {
            let options =
                crate::Options { palettes: false, palette_themed: false, ..Default::default() };
            let Ok((rom, _)) =
                crate::randomize_rom_with_overworld_capture(&rom_bytes, seed, &options, None)
            else {
                continue;
            };
            let map = CompletionMap::from_rom(&rom);

            for w in 0..8 {
                let grid = rom_data::read_tile_grid(&rom, w);
                for row in 0..grid.rows() {
                    for col in 0..grid.cols {
                        if !rom_data::is_lock(grid.get(row, col)) {
                            continue;
                        }
                        let [offset, mask] =
                            away_target(&map, w, (row, col)).unwrap_or_else(|| {
                                panic!("seed {seed} W{} lock at ({row},{col}) owns no bit", w + 1)
                            });

                        let mut region = [0u8; PLANE_RESERVE];
                        region[offset as usize] |= mask;
                        let base = map.base(w);
                        let plane = &region[base..base + map.plane_bytes(w)];

                        let half = map.unpack(w, plane);
                        let want = MAP_COMPLETE_BITS[row.min(7)];
                        assert_eq!(
                            half[col],
                            want,
                            "seed {seed} W{}: the bit expands to column {col} as ${:02X}, \
                             wanted ${want:02X}",
                            w + 1,
                            half[col]
                        );
                        for (c, &b) in half.iter().enumerate() {
                            assert!(
                                c == col || b == 0,
                                "seed {seed} W{}: the bit also lit column {c}",
                                w + 1
                            );
                        }
                        checked += 1;
                    }
                }
            }
        }
        assert!(checked > 0, "no locks in any seed — the census measured nothing");
        eprintln!("{checked} lock cells checked over {} seeds", seeds());
    }
}
