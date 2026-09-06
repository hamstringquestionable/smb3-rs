//! World-maze cross-world locks: a fortress in one world that busts a lock in
//! another.
//!
//! # The whole mechanism is one completion bit
//!
//! `Map_Removable_Tiles` (PRG012) lists `TILE_LOCKVERT`, `TILE_LOCKHORZ` and
//! `TILE_ALTLOCK` next to the rocks, and `Map_Reload_with_Completions` swaps
//! each for its `Map_RemoveTo_Tiles` counterpart wherever the completion bit
//! for that cell is set. That is how vanilla persists a busted lock across a
//! map reload — so a cross-world lock rides the same path and is simply *gone*
//! when its world next loads.
//!
//! Two things follow, and both are savings:
//!
//! - **No tile writing.** Nothing here touches a map grid.
//! - **No FX slot.** The FX system buys the crumble animation plus a bit write
//!   at fort-clear time; a world the player is not standing in needs no
//!   animation. The 17 slots at `FX_MAP_COMP_IDX` therefore bound same-world
//!   locks and bridges only.
//!
//! # The hook: `Map_MarkLevelComplete`'s fortress branch
//!
//! `PRG011_BA7C` is reached only when the tile under the player is
//! `TILE_FORTRUBBLE` (`$60`) or `TILE_ALTRUBBLE` (`$E3`) — a fortress was just
//! beaten. It is a different path from `MO_DoFortressFX`, so same-world lock FX
//! is untouched, and it runs *after* the caller has already swapped the map
//! tile to rubble and queued the crumble sound, so the fortress still crumbles.
//!
//! The branch is four instructions:
//!
//! ```text
//! $BA7C  98        TYA               ; Y is the completion column, + $40 for Luigi
//!        49 40     EOR #$40          ; ... now the OTHER player's half
//!        A8        TAY
//!        B9 00 7D  LDA Map_Completions,Y
//!        1D 2D BA  ORA Map_CompleteBit,X   ; X is the completion row, 0-7
//!        99 00 7D  STA Map_Completions,Y   <- displaced, replayed by us
//! $BA89  60        RTS
//! ```
//!
//! The last `STA` is displaced for a `JSR` — three bytes for three bytes, one
//! whole instruction, and the routine replays it as its own first act. Nothing
//! branches into it (`$BA89`'s `RTS` is the shared exit and is left alone), and
//! the caller reloads `A`, `X` and `Y` immediately after
//! `JSR Map_MarkLevelComplete`, so the routine is free to clobber all three.
//!
//! **Identity is position, not an id.** On arrival `Y` holds the completion
//! column — `(World_Map_XHi << 4) | (World_Map_X >> 4)`, which is exactly the
//! grid column — and `X` holds `Temp_Var13`, the completion row that
//! `Map_CompleteY` resolved (0-7, with map rows 7 and 8 both landing on 7). So
//! a row keys on `(World_Num, column, row)`, the same "where is the player
//! standing" key `PAD_ENTER` uses, and no fort-id numbering has to be invented
//! or kept in sync. `execute_the_row_key_against_the_engine` proves the
//! register claims by running the engine's own bytes rather than trusting this
//! paragraph.
//!
//! # Where the destination bit lives, and why it is precomputed
//!
//! Completions for a world that is **not** loaded live in
//! [`super::completion_bits`]' packed store: two bits per completable cell (one
//! plane per player), at `PACKED` (`$7997`), with each world's slice found
//! through a base *table* and never a stride — World 6 runs to 103 cells
//! against World 1's 21. Which cells own a bit is a stencil derived from the
//! map grid.
//!
//! Computing a bit index on console for a world you are not standing in would
//! mean running `MASK_BUILD` over that world's grid with PRG012 banked in, from
//! inside a PRG011 hook. The randomizer already knows every finished grid, so
//! the stencil walk happens at build time instead and the console side is a
//! scan and two `ORA`s. The build-time half derives the byte and bit **through
//! `CompletionMap::pack` itself** — see [`plane_bit`] — so there is no second
//! copy of that arithmetic to drift.
//!
//! ## The destination is never the live world
//!
//! `LIVE_WORLD` names the world expanded over `$7D00`, and `SWAP_AT_RELOAD`
//! sets it to `World_Num` on every real transition, so on the map they are
//! always equal. The player stands in the fortress's world when this hook
//! fires, and a *foreign* lock is by definition in a different world — so the
//! destination slice is always a parked one, and an `ORA` into it survives
//! until that world is next entered.
//!
//! It has to be that way round: `WIPE_REPLACEMENT` packs `$7D00` over the live
//! world's slice on the way out, so a bit written into the live world's *packed*
//! slice would be overwritten. [`apply`] therefore refuses a same-world pair
//! rather than emitting a row that would be silently undone —
//! `a_same_world_pair_is_not_a_foreign_lock` holds that line. Same-world locks
//! are the FX system's job and already work.
//!
//! # Both planes, always
//!
//! Vanilla mirrors a fortress completion into the other player's half
//! unconditionally — `PRG011_BA7C`'s own comment says "mark complete on both
//! Players (so it remains after Game Over)" — because the game-over merge at
//! `PRG030_9314` ANDs the two halves. A foreign lock that set only Mario's
//! plane would come back after a game over. `Map_Reload_with_Completions` also
//! walks both halves, aliasing them onto the same four screens, so a bit in the
//! wrong plane is not merely lost but shows up as a phantom completed tile
//! somewhere else. Both planes are set, one `PLANE_RESERVE` apart, from the
//! same mask byte.

use crate::rom::Rom;

use super::completion_bits::{CompletionMap, HALF_LEN, PLANE_RESERVE};
#[cfg(test)]
use super::rom_data::NMI_SAFE_MAX;
use super::rom_data::{
    FS_COMPLETION_BASES, FS_MAZE_FOREIGN_LOCK, MAP_COMPLETE_BITS, MAP_COMPLETIONS, WORLD_NUM,
    prg_bank_cpu_to_file, prg011_file_to_cpu,
};

// --- Engine symbols -----------------------------------------------------
//
// `Map_Completions` and `World_Num` are `rom_data::engine`'s; the two zero-page
// scratch bytes are this hook's own argument and are made here.

/// `Temp_Var1` (`$00`) — scratch `Map_MarkLevelComplete` already destroys on
/// this very path (it builds the column into it at `$BA53`), so borrowing it is
/// exactly as safe as vanilla.
const TEMP_VAR1: u8 = 0x00;

/// `Temp_Var13` (`$0C`) — the completion row, stored at `$BA4B` and consumed
/// into `X` at `$BA67`. Dead by the time the hook runs, and destroyed by
/// vanilla on this path either way.
const TEMP_VAR13: u8 = 0x0C;

/// Base of [`super::completion_bits`]' packed store.
///
/// Private over there, so it is restated here and pinned by
/// `the_packed_base_matches_completion_bits`, which reads the address back out
/// of the `PACK_PLANE` bytes that module writes to the ROM. Two modules naming
/// one SRAM address is exactly the drift an assertion is for.
const PACKED: u16 = 0x7997;

/// The mirror plane: one [`PLANE_RESERVE`] on from Mario's.
const PACKED_MIRROR: u16 = PACKED + PLANE_RESERVE as u16;

// --- Siting -------------------------------------------------------------

/// PRG011 is mapped at `$A000` for the whole map, which is where the hook runs.
const FOREIGN_LOCK_CPU: u16 = prg011_file_to_cpu(FS_MAZE_FOREIGN_LOCK);

/// `STA Map_Completions,Y` at CPU `$BA86`, the last instruction of the fortress
/// branch.
const HOOK_OFFSET: usize = prg_bank_cpu_to_file(11, 0xBA86);

/// What stands there in vanilla. Three bytes, one instruction, one `JSR`.
#[cfg(test)]
const HOOK_VANILLA: [u8; 3] = [0x99, 0x00, 0x7D];

/// Bytes reserved by `FS_MAZE_FOREIGN_LOCK`. Pinned to the registry row by
/// `the_reservation_matches_the_registry`.
const RESERVED: usize = 128;

/// Code length; the table follows immediately.
const CODE_LEN: usize = 59;

/// `[key0, key1, plane byte offset, bit mask]`.
const ROW_LEN: usize = 4;

/// How many cross-world locks fit. 17 x 4 = 68, and 59 + 68 = 127 of 128 —
/// which is also the whole lock budget, since the 17 FX slots bound the number
/// of locks a build can place at all.
pub const MAX_ROWS: usize = (RESERVED - CODE_LEN) / ROW_LEN;

/// Where the table starts, in CPU space.
const TABLE_CPU: u16 = FOREIGN_LOCK_CPU + CODE_LEN as u16;

/// Offset of the `LDX #` immediate that sizes the scan.
const COUNT_OPERAND: usize = 15;

/// Set a foreign lock's completion bit when its fortress is beaten.
///
/// Reached by `JSR` from `$BA86` with `A` = the completion byte to store,
/// `Y` = the completion column (plus `$40` for one of the two players) and
/// `X` = the completion row. Replays the displaced store, then scans its table
/// for a row keyed to where the player is standing.
///
/// **The scan does not stop on a match.** A fortress opens exactly one lock
/// today, but falling through into the loop tail costs nothing and means a
/// fortress with two rows would open both.
///
/// Key packing is `row << 3 | World_Num`, and that order is worth a byte:
/// `TXA / ASL x3 / ORA World_Num` builds it in nine bytes where world-first
/// would need a scratch store and twelve. The column compare is `EOR` then
/// `AND #$3F` rather than a masked `CMP`, which moves the mask off the entry
/// path (`STY` is two bytes where `TYA / AND / STA` is five) and, at two bytes
/// per iteration, still comes out one byte ahead.
///
/// 128 reserved, 127 used (59 code + 17 x 4 table).
#[rustfmt::skip]
const FOREIGN_LOCK: [u8; CODE_LEN] = [
    // The displaced instruction, replayed first so nothing observes the gap.
    0x99, MAP_COMPLETIONS as u8, (MAP_COMPLETIONS >> 8) as u8, //  0: STA Map_Completions,Y

    // key0 = row << 3 | World_Num; key1 = the column, player bit and all.
    0x8A,                                                      //  3: TXA        ; row 0-7
    0x0A, 0x0A, 0x0A,                                          //  4: ASL A x3
    0x0D, WORLD_NUM as u8, (WORLD_NUM >> 8) as u8,             //  7: ORA World_Num
    0x85, TEMP_VAR1,                                           // 10: STA <Temp_Var1
    0x84, TEMP_VAR13,                                          // 12: STY <Temp_Var13
    0xA2, 0x00,                                                // 14: LDX #last row  ; patched

    // ----- scan (16) -----
    0xBD, TABLE_CPU as u8, (TABLE_CPU >> 8) as u8,             // 16: LDA KEY0,X
    0xC5, TEMP_VAR1,                                           // 19: CMP <Temp_Var1
    0xD0, 0x1D,                                                // 21: BNE +29 -> next
    0xBD, (TABLE_CPU + 1) as u8, ((TABLE_CPU + 1) >> 8) as u8, // 23: LDA KEY1,X
    0x45, TEMP_VAR13,                                          // 26: EOR <Temp_Var13
    0x29, 0x3F,                                                // 28: AND #$3F   ; drop the player half
    0xD0, 0x14,                                                // 30: BNE +20 -> next

    // ----- hit (32): both planes, from one mask -----
    0xBC, (TABLE_CPU + 2) as u8, ((TABLE_CPU + 2) >> 8) as u8, // 32: LDY OFFSET,X
    0xBD, (TABLE_CPU + 3) as u8, ((TABLE_CPU + 3) >> 8) as u8, // 35: LDA MASK,X
    0x48,                                                      // 38: PHA
    0x19, PACKED as u8, (PACKED >> 8) as u8,                   // 39: ORA PACKED,Y
    0x99, PACKED as u8, (PACKED >> 8) as u8,                   // 42: STA PACKED,Y
    0x68,                                                      // 45: PLA
    0x19, PACKED_MIRROR as u8, (PACKED_MIRROR >> 8) as u8,     // 46: ORA PACKED_MIRROR,Y
    0x99, PACKED_MIRROR as u8, (PACKED_MIRROR >> 8) as u8,     // 49: STA PACKED_MIRROR,Y

    // ----- next (52): rows walked backwards, so the tail is DEX not CPX -----
    0xCA, 0xCA, 0xCA, 0xCA,                                    // 52: DEX x4
    0x10, 0xD6,                                                // 56: BPL -42 -> scan
    0x60,                                                      // 58: RTS
];

/// One cross-world lock, as the maze generator knows it.
///
/// Positions are grid `(row, col)` — the same coordinates
/// `pipe_helpers::grid_pos_to_dest_nibbles` and the map grids use.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ForeignLock {
    /// The world the fortress stands in, 0-based. Where the player will be.
    pub fort_world: usize,
    /// The fortress's map cell.
    pub fort_pos: (usize, usize),
    /// The world the lock stands in, 0-based.
    pub lock_world: usize,
    /// The lock's map cell — whose completion bit gets set.
    pub lock_pos: (usize, usize),
}

/// Where a cell's bit sits in the packed store: `(byte offset from `PACKED`,
/// single-bit mask)`.
///
/// **Derived by running the packer**, not by re-deriving its arithmetic: a
/// `Map_Completions` half with exactly this cell's bit set is packed for the
/// destination world, and whatever bit comes out the other side is the answer.
/// The bit order, the row 7/8 fold and the stencil are then the packer's by
/// construction, and cannot drift from it.
///
/// `None` when the cell owns no bit at all — the stencil says the engine can
/// never act on it, so a lock there could not be persisted by any means.
fn plane_bit(map: &CompletionMap, world: usize, pos: (usize, usize)) -> Option<(u8, u8)> {
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
    Some((offset as u8, mask))
}

/// Assemble the table: one four-byte row per lock the console can act on.
///
/// Silently drops a pair the mechanism cannot serve, because both cases are
/// programming errors rather than seeds:
///
/// - a same-world pair, which is the FX system's job and whose bit would be
///   packed over on the way out of the world (see the module header);
/// - a lock cell the stencil does not own, which no completion bit can reach.
fn table_rows(map: &CompletionMap, locks: &[ForeignLock]) -> Vec<[u8; ROW_LEN]> {
    let mut rows = Vec::with_capacity(locks.len());
    for lock in locks {
        // A same-world lock is not a foreign lock: its bit would be packed over
        // on the way out of the world. See the module header.
        if lock.fort_world == lock.lock_world {
            continue;
        }
        let Some((offset, mask)) = plane_bit(map, lock.lock_world, lock.lock_pos) else {
            debug_assert!(false, "lock cell {:?} owns no completion bit", lock.lock_pos);
            continue;
        };
        debug_assert!(lock.fort_world < 8, "a world index is three bits");
        let key0 = ((lock.fort_pos.0.min(7) as u8) << 3) | lock.fort_world as u8;
        let key1 = lock.fort_pos.1 as u8;
        debug_assert!(key1 < 0x40, "a completion column is six bits");
        rows.push([key0, key1, offset, mask]);
    }
    rows
}

/// Install the cross-world locks: the routine, its table, and the hook.
///
/// Call it **after** the overworld writer has laid down the final map grids and
/// with the same ROM state [`super::completion_bits::apply`] sees — the packed
/// layout is derived from those grids, so a table built against a different
/// version of them names the wrong bits.
///
/// An empty slice leaves the ROM alone, hook included: nothing to open means
/// nothing to hook.
///
/// # Panics
///
/// If more than [`MAX_ROWS`] locks are handed over. Truncating would leave a
/// gate that no fortress opens, which is an unwinnable seed; a build-time
/// failure is the better end of that trade.
pub fn apply(rom: &mut Rom, locks: &[ForeignLock]) {
    if locks.is_empty() {
        return;
    }

    let map = CompletionMap::from_rom(rom);

    // **The ordering guard, and it catches three rules at once.**
    //
    // This module derives each lock's `(plane byte, bit)` by re-reading the map
    // grids, while `completion_bits` has already emitted a base table from the
    // grids as IT saw them. Those two views agree only if the grids have not
    // moved in between — so comparing them here catches, in four lines:
    //
    // * a grid write landing after `completion_bits::apply` ran;
    // * this module running BEFORE it;
    // * `completion_bits` never having run at all, in which case the region is
    //   still `$FF` filler and the compare fails loudly.
    //
    // Every one of those is otherwise silent: the offsets would simply address
    // the wrong cells, and a cross-world lock would open something else. An
    // ordering rule that only a comment knows is a bug waiting for a refactor.
    let emitted = rom.read_range(FS_COMPLETION_BASES, 9);
    assert_eq!(
        emitted,
        map.base_table(),
        "the packed-store base table in the ROM disagrees with the map this module just read. \
         Either a grid was written after `completion_bits::apply`, or this ran before it, or it \
         never ran. See `randomizer::randomize_inner` for the order."
    );

    let rows = table_rows(&map, locks);
    if rows.is_empty() {
        return;
    }
    assert!(
        rows.len() <= MAX_ROWS,
        "{} cross-world locks but only {MAX_ROWS} rows fit `FS_MAZE_FOREIGN_LOCK`",
        rows.len()
    );

    let mut code = Vec::with_capacity(CODE_LEN + rows.len() * ROW_LEN);
    code.extend_from_slice(&FOREIGN_LOCK);
    // The scan walks backwards from the last row's first byte, so the loop tail
    // is `DEX x4 / BPL` rather than a compare against a length.
    code[COUNT_OPERAND] = ((rows.len() - 1) * ROW_LEN) as u8;
    for row in &rows {
        code.extend_from_slice(row);
    }
    rom.write_range(FS_MAZE_FOREIGN_LOCK, &code);

    rom.write_range(HOOK_OFFSET, &hook_bytes());
}

/// The three bytes written over `$BA86`.
fn hook_bytes() -> [u8; 3] {
    [0x20, FOREIGN_LOCK_CPU as u8, (FOREIGN_LOCK_CPU >> 8) as u8]
}

/// Decode the foreign-lock table back out of a finished ROM: one
/// `(fortress world, completion row index, completion column)` per row.
///
/// The layout knowledge lives here rather than in the test that reads it, so a
/// change to the row shape cannot leave a test quietly decoding the old one.
///
/// Note the row is the **completion index**, not the grid row:
/// `Map_MarkLevelComplete` matches `World_Map_Y` against `Map_CompleteY`, which
/// has seven entries, so grid rows 0-6 map to 0-6 and grid rows **7 and 8 both
/// map to 7** — the shared-bit fold. A caller resolving a cell has to try both.
///
/// **"No locks" and "one lock" both read a count of zero**, because the scan
/// walks backwards from `(rows - 1) * 4`. So a count of zero has to be
/// disambiguated by whether the routine is installed at all, and the way to ask
/// that is against [`FOREIGN_LOCK`]'s own first byte — an un-patched ROM holds
/// `$FF` filler there. (`HOOK_VANILLA[0]` would answer the same question today
/// only by coincidence: the routine opens by replaying the instruction it
/// displaced, so the two bytes happen to be the same `$99`.)
#[cfg(test)]
pub(crate) fn decode_rows(rom: &Rom) -> Vec<(usize, usize, usize)> {
    let count = rom.read_byte(FS_MAZE_FOREIGN_LOCK + COUNT_OPERAND) as usize;
    if count == 0 && rom.read_byte(FS_MAZE_FOREIGN_LOCK) != FOREIGN_LOCK[0] {
        return Vec::new();
    }
    let rows = count / ROW_LEN + 1;
    (0..rows)
        .map(|i| {
            let b = FS_MAZE_FOREIGN_LOCK + CODE_LEN + i * ROW_LEN;
            let k0 = rom.read_byte(b) as usize;
            let k1 = rom.read_byte(b + 1) as usize;
            (k0 & 0x07, k0 >> 3, k1 & 0x3F)
        })
        .collect()
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

    /// A full patch with `n` rows, for the structural checks.
    fn assembled(rows: &[[u8; ROW_LEN]]) -> Vec<u8> {
        let mut code = FOREIGN_LOCK.to_vec();
        code[COUNT_OPERAND] = ((rows.len().max(1) - 1) * ROW_LEN) as u8;
        for row in rows {
            code.extend_from_slice(row);
        }
        code
    }

    // --- Structure ------------------------------------------------------

    #[test]
    fn foreign_lock_is_well_formed() {
        let full = assembled(&[[0x00, 0x00, 0x00, 0x80]; MAX_ROWS]);
        assert_eq!(full.len(), 127, "128 reserved, 127 used");
        asm::check(&full)
            .allocation(FS_MAZE_FOREIGN_LOCK)
            .origin(FOREIGN_LOCK_CPU)
            .data_from(CODE_LEN)
            // `Temp_Var13` is above the three the NMI preserves, and is named
            // rather than tolerated: vanilla's own `Map_MarkLevelComplete`
            // holds it live across this very window, so borrowing it is
            // exactly as safe as what it displaced. Anything else in the page
            // would be a bug. See `asm::Routine::zero_page`.
            .zero_page(NMI_SAFE_MAX, &[TEMP_VAR13])
            .assert_ok();
    }

    /// One row is the other extreme: the scan starts at index 0 and the whole
    /// table is four bytes.
    #[test]
    fn a_single_row_is_well_formed() {
        asm::check(&assembled(&[[0x09, 0x04, 0x03, 0x20]]))
            .allocation(FS_MAZE_FOREIGN_LOCK)
            .origin(FOREIGN_LOCK_CPU)
            .data_from(CODE_LEN)
            // `Temp_Var13` is above the three the NMI preserves, and is named
            // rather than tolerated: vanilla's own `Map_MarkLevelComplete`
            // holds it live across this very window, so borrowing it is
            // exactly as safe as what it displaced. Anything else in the page
            // would be a bug. See `asm::Routine::zero_page`.
            .zero_page(NMI_SAFE_MAX, &[TEMP_VAR13])
            .assert_ok();
    }

    /// The hook displaces exactly one whole instruction, and the routine
    /// replays it.
    #[test]
    fn the_hook_displaces_one_whole_instruction() {
        let Some(rom) = vanilla() else {
            eprintln!("SKIP: requires the ROM");
            return;
        };
        assert_eq!(
            rom.read_range(HOOK_OFFSET, HOOK_VANILLA.len()),
            HOOK_VANILLA,
            "the STA at $BA86 has moved"
        );
        // The whole fortress branch, so a shift in *either* direction is loud.
        assert_eq!(
            rom.read_range(HOOK_OFFSET - 10, 14),
            [0x98, 0x49, 0x40, 0xA8, 0xB9, 0x00, 0x7D, 0x1D, 0x2D, 0xBA, 0x99, 0x00, 0x7D, 0x60],
            "PRG011_BA7C is not the fortress branch this hook assumes"
        );
        assert_eq!(
            FOREIGN_LOCK[..3],
            HOOK_VANILLA,
            "the routine must replay the store it displaced"
        );

        let full = assembled(&[[0x09, 0x04, 0x03, 0x20]]);
        asm::check(&full)
            .allocation(FS_MAZE_FOREIGN_LOCK)
            .origin(FOREIGN_LOCK_CPU)
            .data_from(CODE_LEN)
            .zero_page(NMI_SAFE_MAX, &[TEMP_VAR13])
            .hook(&HOOK_VANILLA, 0, &hook_bytes())
            .assert_ok();
    }

    /// The four table reads name this routine's own tail, in order. Get
    /// [`CODE_LEN`] wrong and the scan indexes its own instructions, which
    /// decodes and runs perfectly well.
    #[test]
    fn the_scan_reads_its_own_table() {
        let operand = |at: usize, opcode: u8| {
            assert_eq!(FOREIGN_LOCK[at], opcode, "offset {at} is not the expected indexed load");
            u16::from_le_bytes([FOREIGN_LOCK[at + 1], FOREIGN_LOCK[at + 2]])
        };
        let first = FOREIGN_LOCK_CPU + CODE_LEN as u16;
        for (at, opcode, want, name) in [
            (16, 0xBD, first, "key0"),
            (23, 0xBD, first + 1, "key1"),
            (32, 0xBC, first + 2, "plane offset"),
            (35, 0xBD, first + 3, "bit mask"),
        ] {
            assert_eq!(operand(at, opcode), want, "offset {at} should read {name}");
        }
        assert_eq!(
            FOREIGN_LOCK_CPU, 0xBF6B,
            "the routine moved; every table address above is baked in"
        );
    }

    /// The reservation this module sizes itself against is the registry's.
    #[test]
    fn the_reservation_matches_the_registry() {
        let row = rom_data::FREE_SPACE_ALLOCATIONS
            .iter()
            .find(|a| a.offset == FS_MAZE_FOREIGN_LOCK)
            .expect("FS_MAZE_FOREIGN_LOCK needs a FREE_SPACE_ALLOCATIONS row");
        assert_eq!(row.size, RESERVED);
        const { assert!(CODE_LEN + MAX_ROWS * ROW_LEN <= RESERVED) };
    }

    /// `PACKED` is private to [`completion_bits`], so this module restates it.
    /// Read it back out of the bytes that module writes to the ROM: if the
    /// store ever moves, the two disagree here rather than on a cartridge.
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

    // --- The table's meaning --------------------------------------------

    /// Two locks, one ROM: the emitted rows are the ones `table_rows` computed,
    /// laid out four bytes apart, and the count operand points at the last one.
    #[test]
    fn apply_writes_the_rows_and_the_hook() {
        let Some(rom) = vanilla() else {
            eprintln!("SKIP: requires the ROM");
            return;
        };
        let locks = [
            ForeignLock { fort_world: 0, fort_pos: (4, 5), lock_world: 3, lock_pos: (2, 9) },
            ForeignLock { fort_world: 5, fort_pos: (1, 20), lock_world: 0, lock_pos: (4, 6) },
        ];
        let base = with_locks(&rom, &locks);
        let mut patched = base.clone();
        apply(&mut patched, &locks);

        let map = CompletionMap::from_rom(&base);
        let want = table_rows(&map, &locks);
        assert_eq!(want.len(), 2);
        for (n, row) in want.iter().enumerate() {
            assert_eq!(
                patched.read_range(FS_MAZE_FOREIGN_LOCK + CODE_LEN + n * ROW_LEN, ROW_LEN),
                row,
                "row {n}"
            );
        }
        assert_eq!(
            patched.read_byte(FS_MAZE_FOREIGN_LOCK + COUNT_OPERAND),
            ((want.len() - 1) * ROW_LEN) as u8
        );
        assert_eq!(patched.read_range(HOOK_OFFSET, 3), hook_bytes());
    }

    /// No locks, no patch — the hook stays vanilla so a non-maze seed is
    /// byte-identical here.
    #[test]
    fn cross_world_locks_are_opt_in() {
        let Some(rom) = vanilla() else {
            eprintln!("SKIP: requires the ROM");
            return;
        };
        let mut patched = rom.clone();
        apply(&mut patched, &[]);
        assert_eq!(patched.read_range(HOOK_OFFSET, 3), HOOK_VANILLA);
        assert_eq!(
            patched.read_range(FS_MAZE_FOREIGN_LOCK, RESERVED),
            rom.read_range(FS_MAZE_FOREIGN_LOCK, RESERVED)
        );
    }

    /// A same-world pair is not a foreign lock — its bit would be packed over
    /// on the way out of the world. It never reaches the table.
    #[test]
    fn a_same_world_pair_is_not_a_foreign_lock() {
        let Some(rom) = vanilla() else {
            eprintln!("SKIP: requires the ROM");
            return;
        };
        let map = CompletionMap::from_rom(&rom);
        let rows = table_rows(
            &map,
            &[ForeignLock { fort_world: 2, fort_pos: (3, 4), lock_world: 2, lock_pos: (5, 6) }],
        );
        assert!(rows.is_empty(), "the live world can never be a foreign lock's destination");
    }

    /// **The payload, checked by expanding it rather than by recomputing it.**
    ///
    /// A row says "set this bit of this world's plane". Set exactly that bit in
    /// an otherwise empty store, run [`CompletionMap::unpack`] — the *other*
    /// direction, and the one the console runs on arrival — and the resulting
    /// `Map_Completions` half must carry the lock cell's bit and nothing else.
    ///
    /// Over real builds, on every lock every seed actually placed.
    ///
    /// ```sh
    /// CENSUS_SEEDS=200 cargo test --release --lib the_table_expands_to_the_lock_cell
    /// ```
    #[test]
    fn the_table_expands_to_the_lock_cell() {
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
                        // Any fortress in another world; only the payload is
                        // under test here, and the key has its own oracle.
                        let lock = ForeignLock {
                            fort_world: (w + 1) % 8,
                            fort_pos: (0, 2),
                            lock_world: w,
                            lock_pos: (row, col),
                        };
                        let rows = table_rows(&map, &[lock]);
                        assert_eq!(
                            rows.len(),
                            1,
                            "seed {seed} W{} lock at ({row},{col}) owns no bit",
                            w + 1
                        );
                        let [_, _, offset, mask] = rows[0];

                        // The console's write, replayed: one bit, one plane.
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

    // --- Executing it ---------------------------------------------------
    //
    // `asm::check` proves the bytes decode; it cannot see that they compute the
    // right answer. This routine calls nothing — it reads `World_Num`, its own
    // table and two zero-page scratch bytes — so run it, and run it from
    // `Map_MarkLevelComplete`'s own entry rather than from the hook, which
    // makes the engine itself supply `X` and `Y` instead of this test asserting
    // what the disassembly says they hold.

    /// `Map_MarkLevelComplete`, PRG011.
    const MARK_LEVEL_COMPLETE: u16 = 0xBA35;
    /// Where the harness parks its return address.
    const SENTINEL: u16 = 0x0F00;
    /// `Map_Player_SkidBack` — two bytes, `$073E`.
    const SKID_BACK: u16 = 0x073E;
    /// `Player_Current`.
    const PLAYER_CURRENT: u16 = 0x0726;
    /// `World_Map_Tile`, zero page.
    const WORLD_MAP_TILE: u16 = 0x00E5;
    /// `TILE_FORTRUBBLE`.
    const FORT_RUBBLE: u8 = 0x60;

    /// A CPU with PRG011 mapped at `$A000` and the player standing on a beaten
    /// fortress at `at`, in `world`, as `player`.
    fn fort_cleared(
        rom: &Rom,
        world: u8,
        player: u8,
        at: (usize, usize),
        tile: u8,
    ) -> CPU<Memory, Ricoh2a03> {
        let mut mem = Memory::new();
        // The whole bank, so the routine, the hook and `Map_CompleteBit` are
        // all the real thing.
        mem.set_bytes(0xA000, rom.read_range(0x16010, 0x2000));

        mem.set_byte(PLAYER_CURRENT, player);
        mem.set_byte(SKID_BACK + player as u16, 0);
        mem.set_byte(WORLD_NUM, world);
        mem.set_byte(WORLD_MAP_TILE, tile);

        // The engine's own encoding of a grid cell: row (grid_row + 2) << 4,
        // column split across screen and pixel X.
        let (screen, col, row_nib) =
            crate::randomize::pipe_helpers::grid_pos_to_dest_nibbles(at.0, at.1);
        mem.set_byte(0x0075 + player as u16, row_nib << 4);
        mem.set_byte(0x0077 + player as u16, screen);
        mem.set_byte(0x0079 + player as u16, col << 4);

        CPU::new(mem, Ricoh2a03)
    }

    /// Run `Map_MarkLevelComplete` to its `RTS`.
    fn run(cpu: &mut CPU<Memory, Ricoh2a03>) {
        let ret = SENTINEL.wrapping_sub(1);
        cpu.memory.set_byte(0x01FF, (ret >> 8) as u8);
        cpu.memory.set_byte(0x01FE, ret as u8);
        cpu.registers.stack_pointer = mos6502::registers::StackPointer(0xFD);
        cpu.registers.program_counter = MARK_LEVEL_COMPLETE;
        for _ in 0..10_000 {
            if cpu.registers.program_counter == SENTINEL {
                return;
            }
            cpu.single_step();
        }
        panic!("Map_MarkLevelComplete ran away");
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

    /// Stamp lock tiles into the grids, so the destination cells own a
    /// completion bit.
    ///
    /// The stencil is derived from the map grid, and vanilla has no lock at an
    /// arbitrary cell — a test that skipped this would be measuring a table row
    /// for a cell the engine can never mark.
    fn with_locks(rom: &Rom, locks: &[ForeignLock]) -> Rom {
        let mut out = rom.clone();
        for lock in locks {
            out.write_byte(
                rom_data::map_tile_offset(lock.lock_world, lock.lock_pos.0, lock.lock_pos.1),
                rom_data::LOCK_TILES[0],
            );
        }
        // Install the packed store LAST, exactly as the pipeline does: every
        // grid write has to precede it, and  now asserts that the base
        // table in the ROM still describes the map it reads. A fixture that
        // skipped this would be testing  against a precondition the
        // real pipeline never presents it with.
        completion_bits::apply(&mut out);
        out
    }

    /// A ROM with one cross-world lock installed, plus the row it emitted.
    fn one_lock(rom: &Rom, lock: ForeignLock) -> (Rom, [u8; ROW_LEN]) {
        let base = with_locks(rom, &[lock]);
        let mut patched = base.clone();
        apply(&mut patched, &[lock]);
        let map = CompletionMap::from_rom(&base);
        (patched, table_rows(&map, &[lock])[0])
    }

    /// **The headline: a fortress in one world sets a lock's bit in another.**
    ///
    /// Both planes, exactly one bit each, and nothing else in the store — and
    /// the live world's own `Map_Completions` still gets vanilla's two marks.
    #[test]
    fn a_foreign_fortress_sets_exactly_one_bit_in_each_plane() {
        let Some(rom) = vanilla() else {
            eprintln!("SKIP: requires the ROM");
            return;
        };
        let lock = ForeignLock { fort_world: 0, fort_pos: (4, 5), lock_world: 3, lock_pos: (2, 9) };
        let (patched, row) = one_lock(&rom, lock);
        let [_, _, offset, mask] = row;

        let mut cpu = fort_cleared(&patched, lock.fort_world as u8, 0, lock.fort_pos, FORT_RUBBLE);
        run(&mut cpu);

        assert_eq!(
            packed(&mut cpu),
            vec![(PACKED + offset as u16, mask), (PACKED_MIRROR + offset as u16, mask),],
            "both planes, one bit each, nothing else"
        );

        // Vanilla's own work is untouched: the fortress cell is marked in both
        // halves of the live world's array.
        assert_eq!(cpu.memory.get_byte(MAP_COMPLETIONS + 5), 0x08, "row 4 of column 5, Mario");
        assert_eq!(cpu.memory.get_byte(MAP_COMPLETIONS + 0x40 + 5), 0x08, "and the mirror");
    }

    /// Luigi's clear must set the same foreign bit. The column key carries a
    /// `$40` for one of the two players and the branch flips it again, so the
    /// scan has to compare six bits and not eight.
    #[test]
    fn either_player_opens_the_same_lock() {
        let Some(rom) = vanilla() else {
            eprintln!("SKIP: requires the ROM");
            return;
        };
        let lock =
            ForeignLock { fort_world: 6, fort_pos: (6, 33), lock_world: 1, lock_pos: (3, 12) };
        let (patched, row) = one_lock(&rom, lock);
        let [_, _, offset, mask] = row;

        for player in 0..2u8 {
            let mut cpu =
                fort_cleared(&patched, lock.fort_world as u8, player, lock.fort_pos, FORT_RUBBLE);
            run(&mut cpu);
            assert_eq!(
                packed(&mut cpu),
                vec![(PACKED + offset as u16, mask), (PACKED_MIRROR + offset as u16, mask)],
                "player {player}"
            );
        }
    }

    /// **The row key, against the engine's own `Map_CompleteY` table.**
    ///
    /// Every map row, both ends of the map, a sub-tile pixel offset on the
    /// column: the key `apply` writes must be the one the engine computes. Map
    /// rows 7 and 8 both fall through to completion index 7 — that is the
    /// engine's shared-bit rule, and a key that did not fold them would miss
    /// half the forts on the bottom row.
    #[test]
    fn execute_the_row_key_against_the_engine() {
        let Some(rom) = vanilla() else {
            eprintln!("SKIP: requires the ROM");
            return;
        };
        for row in 0..9usize {
            for col in [0usize, 5, 16, 47] {
                let lock = ForeignLock {
                    fort_world: 4,
                    fort_pos: (row, col),
                    lock_world: 0,
                    lock_pos: (2, 4),
                };
                let (patched, emitted) = one_lock(&rom, lock);
                let [_, _, offset, mask] = emitted;

                let mut cpu = fort_cleared(&patched, 4, 0, (row, col), FORT_RUBBLE);
                run(&mut cpu);
                assert_eq!(
                    packed(&mut cpu),
                    vec![(PACKED + offset as u16, mask), (PACKED_MIRROR + offset as u16, mask)],
                    "row {row} column {col} did not match its own key"
                );
            }
        }
    }

    /// Standing anywhere else — wrong world, wrong row, wrong column, or a
    /// tile that is not rubble — sets nothing.
    #[test]
    fn a_non_matching_position_sets_nothing() {
        let Some(rom) = vanilla() else {
            eprintln!("SKIP: requires the ROM");
            return;
        };
        let lock =
            ForeignLock { fort_world: 2, fort_pos: (4, 5), lock_world: 7, lock_pos: (6, 30) };
        let (patched, _) = one_lock(&rom, lock);

        for (world, at, tile, why) in [
            (3u8, (4usize, 5usize), FORT_RUBBLE, "another world"),
            (2, (5, 5), FORT_RUBBLE, "another row"),
            (2, (4, 6), FORT_RUBBLE, "another column"),
            (2, (4, 21), FORT_RUBBLE, "the same column on another screen"),
            (2, (4, 5), 0x03, "a level panel, not a fortress"),
        ] {
            let mut cpu = fort_cleared(&patched, world, 0, at, tile);
            run(&mut cpu);
            assert!(packed(&mut cpu).is_empty(), "{why} still set a foreign bit");
        }
    }

    /// A fortress cleared while skidding back is not a clear at all, and
    /// `Map_MarkLevelComplete` bails before the hook. Cheap to check, and it is
    /// the one path that reaches `$BA89`'s `RTS` without passing through us.
    #[test]
    fn a_skid_back_sets_nothing() {
        let Some(rom) = vanilla() else {
            eprintln!("SKIP: requires the ROM");
            return;
        };
        let lock =
            ForeignLock { fort_world: 2, fort_pos: (4, 5), lock_world: 7, lock_pos: (6, 30) };
        let (patched, _) = one_lock(&rom, lock);
        let mut cpu = fort_cleared(&patched, 2, 0, (4, 5), FORT_RUBBLE);
        cpu.memory.set_byte(SKID_BACK, 1);
        run(&mut cpu);
        assert!(packed(&mut cpu).is_empty());
        assert_eq!(cpu.memory.get_byte(MAP_COMPLETIONS + 5), 0, "and nothing was marked at all");
    }

    /// Several locks in one table, and the right one wins.
    #[test]
    fn the_scan_picks_the_row_it_is_standing_on() {
        let Some(rom) = vanilla() else {
            eprintln!("SKIP: requires the ROM");
            return;
        };
        let locks: Vec<ForeignLock> = (0..MAX_ROWS)
            .map(|n| ForeignLock {
                fort_world: n % 8,
                fort_pos: (n % 7, 3 + n),
                lock_world: (n + 3) % 8,
                lock_pos: (1 + n % 5, 2 + n),
            })
            .collect();
        let base = with_locks(&rom, &locks);
        let mut patched = base.clone();
        apply(&mut patched, &locks);
        let map = CompletionMap::from_rom(&base);
        let rows = table_rows(&map, &locks);
        assert_eq!(rows.len(), MAX_ROWS, "the table must be full for this to mean anything");

        for (n, lock) in locks.iter().enumerate() {
            let [_, _, offset, mask] = rows[n];
            let mut cpu =
                fort_cleared(&patched, lock.fort_world as u8, 0, lock.fort_pos, FORT_RUBBLE);
            run(&mut cpu);
            let got = packed(&mut cpu);
            assert!(
                got.contains(&(PACKED + offset as u16, mask))
                    && got.contains(&(PACKED_MIRROR + offset as u16, mask)),
                "row {n} did not fire: {got:02X?}"
            );
        }
    }

    // --- Mutations ------------------------------------------------------
    //
    // Every assertion above passes on a routine that is subtly wrong unless
    // something proves otherwise. These plant the wrongness and demand a
    // failure.

    /// Run the lock from `a_foreign_fortress_sets_exactly_one_bit_in_each_plane`
    /// over a ROM some `mutate` has damaged, and report what the store ended up
    /// holding.
    fn mutated(rom: &Rom, mutate: impl FnOnce(&mut Rom)) -> (Vec<(u16, u8)>, [u8; ROW_LEN]) {
        let lock = ForeignLock { fort_world: 0, fort_pos: (4, 5), lock_world: 3, lock_pos: (2, 9) };
        let (mut patched, row) = one_lock(rom, lock);
        mutate(&mut patched);
        let mut cpu = fort_cleared(&patched, lock.fort_world as u8, 0, lock.fort_pos, FORT_RUBBLE);
        run(&mut cpu);
        (packed(&mut cpu), row)
    }

    #[test]
    fn a_wrong_bit_mask_is_caught() {
        let Some(rom) = vanilla() else {
            eprintln!("SKIP: requires the ROM");
            return;
        };
        let (got, row) = mutated(&rom, |r| {
            let at = FS_MAZE_FOREIGN_LOCK + CODE_LEN + 3;
            let mask = r.read_byte(at);
            r.write_byte(at, if mask == 0x80 { 0x40 } else { mask << 1 });
        });
        let [_, _, offset, mask] = row;
        assert!(!got.is_empty(), "the mutated routine wrote nothing at all");
        assert_ne!(
            got,
            vec![(PACKED + offset as u16, mask), (PACKED_MIRROR + offset as u16, mask)],
            "a planted wrong bit mask survived"
        );
    }

    #[test]
    fn a_wrong_byte_offset_is_caught() {
        let Some(rom) = vanilla() else {
            eprintln!("SKIP: requires the ROM");
            return;
        };
        let (got, row) = mutated(&rom, |r| {
            let at = FS_MAZE_FOREIGN_LOCK + CODE_LEN + 2;
            let off = r.read_byte(at);
            r.write_byte(at, off + 1);
        });
        let [_, _, offset, mask] = row;
        assert!(!got.is_empty(), "the mutated routine wrote nothing at all");
        assert_ne!(
            got,
            vec![(PACKED + offset as u16, mask), (PACKED_MIRROR + offset as u16, mask)],
            "a planted wrong plane offset survived"
        );
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
        let (got, row) = mutated(&rom, |r| {
            // The `STA PACKED_MIRROR,Y` at code offset 49.
            r.write_range(FS_MAZE_FOREIGN_LOCK + 49, &[0x99, PACKED as u8, (PACKED >> 8) as u8]);
        });
        let [_, _, offset, mask] = row;
        assert_eq!(
            got,
            vec![(PACKED + offset as u16, mask)],
            "the mirror plane was never really written"
        );
    }

    /// Widen the column compare to eight bits and Luigi stops matching, because
    /// his half of the key carries `$40`.
    #[test]
    fn comparing_the_player_bit_is_caught() {
        let Some(rom) = vanilla() else {
            eprintln!("SKIP: requires the ROM");
            return;
        };
        let lock =
            ForeignLock { fort_world: 6, fort_pos: (6, 33), lock_world: 1, lock_pos: (3, 12) };
        let (mut patched, row) = one_lock(&rom, lock);
        patched.write_byte(FS_MAZE_FOREIGN_LOCK + 29, 0xFF); // AND #$3F -> AND #$FF
        let [_, _, offset, mask] = row;

        let hits: Vec<bool> = (0..2u8)
            .map(|player| {
                let mut cpu = fort_cleared(
                    &patched,
                    lock.fort_world as u8,
                    player,
                    lock.fort_pos,
                    FORT_RUBBLE,
                );
                run(&mut cpu);
                packed(&mut cpu)
                    == vec![(PACKED + offset as u16, mask), (PACKED_MIRROR + offset as u16, mask)]
            })
            .collect();
        assert_ne!(hits, vec![true, true], "the player bit was never really masked off");
    }
}
