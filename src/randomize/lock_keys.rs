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
    self, FS_COMPLETION_BASES, FS_FORTRESS_FX, FS_LOCK_ENTRIES, FS_LOCK_MIRROR,
    MAP_COMPLETE_BIT_CPU, MAP_COMPLETE_BITS, MAP_COMPLETIONS, PLAYER_CURRENT, PRG012_FILE_BASE,
    WORLD_NUM, prg010_file_to_cpu, prg011_file_to_cpu,
};

// --- Siting -------------------------------------------------------------

/// The mirror sits in PRG011, in the run the retired `Map_MarkLevelComplete`
/// hook gave back. Both map banks are mapped for the whole map (`$84A0` sets
/// PAGE_A000 = 11 and PAGE_C000 = 10), so PRG010 code reads it freely — and
/// keeping it out of PRG010 leaves the whole freed fortress-FX block to the
/// routine, which is the thing that grows.
const MIRROR_CPU: u16 = prg011_file_to_cpu(FS_LOCK_MIRROR);
/// `Map_Removable_Tiles`, mirrored: 8 obstacle tiles.
const MIRROR_REMOVABLE: u16 = MIRROR_CPU;
/// `Map_RemoveTo_Tiles`, mirrored: what each obstacle becomes.
const MIRROR_REMOVE_TO: u16 = MIRROR_CPU + 8;
/// Four CHR quadrants per remove-to tile, in the order the effect queues them.
const MIRROR_PATTERNS: u16 = MIRROR_CPU + 16;
/// Bytes the mirror occupies.
const MIRROR_LEN: usize = 48;

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

/// `Map_Removable_Tiles` / `Map_RemoveTo_Tiles`, CPU `$A437`/`$A43F` in PRG012,
/// 8 parallel entries each. The engine's own answer to "what does this obstacle
/// become", used by `Map_Reload_with_Completions` on every map load.
const MAP_REMOVABLE_TILES: usize = PRG012_FILE_BASE + 0x437;
const MAP_REMOVE_TO_TILES: usize = PRG012_FILE_BASE + 0x43F;
const REMOVABLE_COUNT: usize = 8;

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
    0xA2, 0x07,                                    // 175: LDX #$07
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
    for gap in [0x54u8, 0x56, 0xE4, rom_data::WATER_GAP_TILE] {
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
        let Some(rom) = vanilla() else {
            eprintln!("SKIP: requires the ROM");
            return;
        };
        let mirror = mirror_bytes(&rom);
        assert_mirror_agrees_with_rust(&mirror);

        // Vanilla's eight, spelled out. A change here means the engine's
        // vocabulary of obstacles moved, which is the stage 3 project.
        assert_eq!(
            &mirror[..8],
            &[0x51, 0x52, 0x54, 0x67, 0xEB, 0xE4, 0x56, 0x9D],
            "Map_Removable_Tiles is not where or what this expects"
        );
        assert_eq!(
            &mirror[8..16],
            &[0x45, 0x46, 0x46, 0x60, 0xE3, 0xDA, 0x45, 0xB3],
            "Map_RemoveTo_Tiles is not where or what this expects"
        );
        for i in 0..REMOVABLE_COUNT {
            let to = mirror[REMOVABLE_COUNT + i] as usize;
            let want: [u8; 4] =
                PATTERN_QUADRANT_ORDER.map(|q| rom.read_byte(PRG012_FILE_BASE + q * 256 + to));
            assert_eq!(
                &mirror[16 + i * 4..20 + i * 4],
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
