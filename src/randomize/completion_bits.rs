//! World-maze phase 1: which map cells own a completion bit, and where each
//! world's packed slice lives.
//!
//! # Why this exists
//!
//! `Map_Completions` is 128 bytes at `$7D00` — 64 columns of eight row-bits for
//! Mario, then the same again for the permanent-alteration mirror — and it
//! holds exactly *one* world. The world-maze needs eight worlds resident at
//! once, and 8 x 128 = 1024 bytes against the 384 bytes of free SRAM the
//! disassembly declares. Raw banking fits two worlds; a third has nowhere to
//! go. Packing is what makes a third world possible, not an optimisation.
//!
//! The saving is that almost none of those 512 bits can ever be set. A world's
//! grid is mostly path and scenery; only the cells
//! `Map_Reload_with_Completions` can act on — level panels, forts, toad houses,
//! locks, rocks, water gaps — ever carry a bit. Across seeds that is 21 to 103
//! cells per world (`completion_bit_census`), so one bit per *owning cell*
//! instead of one bit per (column, row) turns 128 bytes per world into ~6 to
//! ~13.
//!
//! # The contract
//!
//! A world's **column mask** is the whole model: one byte per map column, with
//! the same bit layout as `Map_Completions` itself, set wherever that cell owns
//! a bit. Packing is then "walk the mask, take the live bits in order"; the
//! mask is also what a 6502 twin has to reproduce, whether it derives the mask
//! by walking the ROM grid or reads one the randomizer emitted.
//!
//! Bit order within a plane is the engine's own: columns ascending, and within
//! a column `$80` down to `$01` — so a 6502 packer is a `ROL` accumulator over
//! the same loop the engine already runs in `PRG012_A4E5`.
//!
//! ## Row 7 and row 8 share bit `$01`
//!
//! `Map_Complete_Bits` has eight entries for nine map rows, and
//! `Map_MarkLevelComplete`'s `Map_CompleteY` table has seven Y coordinates for
//! the same nine rows, falling back to index 7 for anything it does not
//! recognise. Both rows 7 and 8 therefore write bit `$01`. Reading it back,
//! `PRG012_A55C` steps down to row 8 only when row 7's tile is not one it can
//! act on — so the bit means row 7 whenever row 7 is completable, and row 8
//! otherwise.
//!
//! The mask folds the pair into the one bit they share, which is faithful: one
//! bit is one bit however many tiles claim it. Keeping two pieces of *content*
//! off the pair is the builder's job, not this module's —
//! `WorldState::row78_barred` does it, over placed slots, locks and completable
//! terrain alike. `row78_collision_census` below watches that from this side.
//!
//! # Two planes, not one
//!
//! Mario's half and the mirror at `$7D40` are both stored. The mirror is not a
//! copy: three sites write a permanent alteration into it unconditionally so it
//! survives a game over, `Map_Reload_with_Completions` applies it as a second
//! pass over the same four screens, and the game-over merge at `PRG030_9314`
//! ANDs the two. Banking one and dropping the other is what made a World 1
//! fortress light a tile in World 2 (see [`super::world_persist`]). Each plane
//! is stored separately and byte-aligned per world: all eight Mario planes
//! first, then all eight mirror planes at a fixed offset.
//!
//! # What this module does not do
//!
//! It emits the tables; it does not write them to the ROM and it does not
//! replace the two-world swap at `$84CD`. That is the next step, and it needs
//! the 6502 twin this module is the reference for.

// Reason: every item here is the reference twin for the 6502 pack/unpack that
// replaces the two-world swap at `$84CD`, plus the tables that patch will read.
// The tests below exercise all of it; the ROM-writing consumer lands with the
// 6502 side.
#![allow(dead_code)]

use crate::rom::Rom;

use super::overworld_build::is_completion_unsafe;
use super::rom_data::{self, Grid, MAP_COMPLETE_BITS};

/// One `Map_Completions` half — 64 columns, one byte of row-bits each.
pub(crate) const HALF_LEN: usize = 64;

/// Which cells of one world's map own a completion bit, and where every
/// world's packed slice starts.
///
/// Built from the ROM's own tile grids, so it describes the map a given seed
/// actually produced rather than vanilla's layout.
#[derive(Clone, Debug)]
pub(crate) struct CompletionMap {
    /// Per world, one mask byte per map column, in `Map_Completions` bit order.
    masks: [Vec<u8>; 8],
    /// Byte offset of each world's plane within one plane region, plus a
    /// terminator: `bases[w + 1] - bases[w]` is world `w`'s plane length and
    /// `bases[8]` is the whole region's length.
    bases: [u8; 9],
}

impl CompletionMap {
    /// Read every world's grid and work out its owning cells.
    ///
    /// Reads the **ROM** grids, never `Tile_Mem`: the live copy mutates as you
    /// play — a cleared level becomes an M/L tile, a busted lock becomes path —
    /// so enumerating it would shift every bit after the first change.
    pub(crate) fn from_rom(rom: &Rom) -> Self {
        let masks: [Vec<u8>; 8] =
            std::array::from_fn(|w| world_mask(&rom_data::read_tile_grid(rom, w)));

        let mut bases = [0u8; 9];
        for w in 0..8 {
            let bytes = popcount(&masks[w]).div_ceil(8);
            bases[w + 1] = bases[w]
                .checked_add(bytes as u8)
                .expect("packed completion planes must fit a one-byte offset table");
        }

        Self { masks, bases }
    }

    /// World `w`'s column mask — one byte per map column.
    pub(crate) fn mask(&self, world: usize) -> &[u8] {
        &self.masks[world]
    }

    /// How many bits world `w` needs in one plane.
    pub(crate) fn bits(&self, world: usize) -> usize {
        popcount(&self.masks[world])
    }

    /// Byte length of world `w`'s slice of one plane.
    pub(crate) fn plane_bytes(&self, world: usize) -> usize {
        (self.bases[world + 1] - self.bases[world]) as usize
    }

    /// Byte offset of world `w`'s Mario plane from the start of the packed
    /// region. Its mirror plane sits [`Self::mirror_offset`] further on.
    pub(crate) fn base(&self, world: usize) -> usize {
        self.bases[world] as usize
    }

    /// Distance from a world's Mario plane to its mirror plane — the length of
    /// the whole Mario region, since the planes are stored one after the other.
    pub(crate) fn mirror_offset(&self) -> usize {
        self.bases[8] as usize
    }

    /// Total SRAM the packed region needs, both planes.
    pub(crate) fn total_bytes(&self) -> usize {
        2 * self.mirror_offset()
    }

    /// The nine-byte table a 6502 twin indexes by world.
    ///
    /// Nine and not eight so a world's length comes for free as
    /// `table[w + 1] - table[w]`, and `table[8]` doubles as the Mario-to-mirror
    /// distance. The offsets are a table and never a stride — World 6 runs to
    /// 103 cells against World 1's 21.
    pub(crate) fn base_table(&self) -> [u8; 9] {
        self.bases
    }

    /// Compress one `Map_Completions` half into world `w`'s plane.
    ///
    /// Bits the mask does not own are dropped, which is lossless by
    /// construction: the engine only ever sets a bit through a cell it acted
    /// on, and those are exactly the owned ones.
    pub(crate) fn pack(&self, world: usize, half: &[u8]) -> Vec<u8> {
        let mut out = vec![0u8; self.plane_bytes(world)];
        let mut k = 0;
        for (col, &m) in self.masks[world].iter().enumerate() {
            for bit in MAP_COMPLETE_BITS {
                if m & bit == 0 {
                    continue;
                }
                if half[col] & bit != 0 {
                    out[k / 8] |= 0x80 >> (k % 8);
                }
                k += 1;
            }
        }
        out
    }

    /// Expand world `w`'s plane back into a full `Map_Completions` half.
    pub(crate) fn unpack(&self, world: usize, plane: &[u8]) -> [u8; HALF_LEN] {
        let mut half = [0u8; HALF_LEN];
        let mut k = 0;
        for (col, &m) in self.masks[world].iter().enumerate() {
            for bit in MAP_COMPLETE_BITS {
                if m & bit == 0 {
                    continue;
                }
                if plane[k / 8] & (0x80 >> (k % 8)) != 0 {
                    half[col] |= bit;
                }
                k += 1;
            }
        }
        half
    }
}

/// One world's column mask.
///
/// Rows 0..=7 take their own bit; row 8 folds onto row 7's `$01`, which is the
/// engine's rule and not an approximation — see the module header.
fn world_mask(grid: &Grid) -> Vec<u8> {
    let mut mask = vec![0u8; grid.cols];
    for (col, m) in mask.iter_mut().enumerate() {
        for row in 0..grid.rows() {
            if is_completion_unsafe(grid.get(row, col)) {
                *m |= MAP_COMPLETE_BITS[row.min(7)];
            }
        }
    }
    mask
}

fn popcount(mask: &[u8]) -> usize {
    mask.iter().map(|b| b.count_ones() as usize).sum()
}

// ---------------------------------------------------------------------------
// The console side
// ---------------------------------------------------------------------------
//
// Everything above runs on the build machine. The NES has to reach the same
// answer, and it cannot be handed the stencil without spending ROM on it — 304
// bytes raw, 204 packed, per seed, in a bank whose largest gap is 308. So it
// derives the stencil instead, from the map grid PRG012 already holds and the
// same three tables the engine's own completion pass consults.
//
// **The hook is `JSR Map_Reload_with_Completions` at CPU `$85BB`, not the
// `Map_Completions` wipe.** The wipe is 14 bytes earlier in `PRG030_84A0` and
// looks like the natural site, but PRG011 is at `$A000` there. Four
// instructions later `$84A0` banks PRG012 in for the reload:
//
// ```text
// $85B3   A9 0C       LDA #12
// $85B5   8D 20 07    STA PAGE_A000
// $85B8   20 C2 FF    JSR PRGROM_Change_A000
// $85BB   20 5D A4    JSR Map_Reload_with_Completions   <- here
// ```
//
// which is the only point in the init where the grid *and* the tile tables
// *and* PRG010 (still at `$C000`, holding this code) are all reachable at once.

use super::rom_data::{FS_IS_COMPLETABLE, FS_MASK_BUILD, FS_WORLD_COLS};

/// Where the derived stencil lands: 64 bytes, one per possible map column.
///
/// Sits at the head of `$7A73-$7ADF`, the largest of the disassembly's
/// anonymous `.ds` runs and one the `world_persist` POC already proved free at
/// runtime by parking a completion bank there. The POC's use of it is
/// superseded by the packed region, not shared with it.
const MASK_SCRATCH: u16 = 0x7A73;
/// Running output index, which is also the absolute map column.
const COL_IDX: u16 = 0x7AB3;
/// Columns left to walk in this world.
const COLS_LEFT: u16 = 0x7AB4;

/// PRG010 is mapped at `$C000` for the whole world-map init, so a file offset
/// in it is `$C000 + (file - 0x14010)` — the same arithmetic `world_persist`,
/// `map_warp` and `canoe_summon` use.
const fn prg010_cpu(file: usize) -> u16 {
    (0xC000 + (file - 0x14010)) as u16
}

const MASK_BUILD_CPU: u16 = prg010_cpu(FS_MASK_BUILD);
const IS_COMPLETABLE_CPU: u16 = prg010_cpu(FS_IS_COMPLETABLE);
const WORLD_COLS_CPU: u16 = prg010_cpu(FS_WORLD_COLS);

// --- PRG012 symbols, all verified by matching their bytes in the ROM rather
// --- than read off the disassembly's labels.

/// `Tile_Attributes_TS0` — four "lowest enterable tile" thresholds, indexed by
/// the tile's top two bits. `03 67 BF E9`.
const TILE_ATTRIBUTES_TS0: u16 = 0xA400;
/// `Map_Removable_Tiles` — 8 entries: the two rocks, three locks, two fortress
/// variants and the water gap.
const MAP_REMOVABLE_TILES: u16 = 0xA437;
/// `Map_Completable_Tiles` — 5 entries the engine marks with an M/L outright:
/// both toad houses, the spade panel, the hand trap and the dancing flower.
const MAP_COMPLETABLE_TILES: u16 = 0xA447;
/// `Map_Tile_Layouts` — one 16-bit pointer per world to its tile grid.
const MAP_TILE_LAYOUTS: u16 = 0xA598;

// --- Zero page.
//
// Only `Temp_Var1..5`, and deliberately: `Map_Reload_with_Completions` — the
// very next thing to run — clobbers exactly those five itself. Anything that
// needed them to survive across it is already broken in vanilla, so borrowing
// them here cannot break something that works. The wider `Temp_Var6..16` carry
// no such guarantee. See [[zero_page_not_free]]: "unused" in the disassembly is
// per-context, and this is the context that matters.

/// `Temp_Var1`/`Temp_Var2` — the 16-bit pointer at the current map screen.
/// Indirect-indexed addressing has no other option than zero page.
const ZP_GRID: u8 = 0x00;
/// `Temp_Var3` — the mask byte being assembled for the current column.
const ZP_MASK: u8 = 0x02;
/// `Temp_Var4` — the row bit, walking `$80` down to `$01`.
const ZP_BIT: u8 = 0x03;
/// `Temp_Var5` — the tile, stashed so the threshold test can index on its top
/// two bits and still compare against the whole byte.
const ZP_TILE: u8 = 0x04;

/// Map columns per world. Fixed ROM geometry — one, two, three or four screens
/// of sixteen — and not something the randomizer moves, so it is a constant
/// table rather than emitted per seed.
///
/// There is no engine table to borrow. `World_Map_Max_PanR` looks like one and
/// is not: Worlds 5 and 8 are `$00` there because they never pan, and its own
/// comment says movement is restricted by separate lock-out code instead.
#[rustfmt::skip]
pub(crate) const WORLD_COLS: [u8; 8] = [16, 32, 48, 32, 32, 48, 32, 64];

/// Is this tile one the engine's completion pass acts on? Carry set if so.
///
/// The three tests are the engine's own, in its own order, reading its own
/// tables at their real addresses — which is the whole reason this routine is
/// cheaper than shipping a stencil. It is the 6502 spelling of
/// [`is_completion_unsafe`], and the equivalence test holds the two together.
///
/// The fortress tiles need no case of their own: `$67` and `$EB` are entries in
/// `Map_Removable_Tiles`, and `$6A` clears the page-1 threshold.
///
/// `A` is the tile on entry and is destroyed; `Y` is preserved, which is what
/// lets the caller keep its row offset across the call.
#[rustfmt::skip]
const IS_COMPLETABLE: [u8; 37] = [
    // --- Map_Completable_Tiles: marked with an M/L outright ---
    0xA2, 0x04,                             //  0: LDX #4
    0xDD, MAP_COMPLETABLE_TILES as u8,
          (MAP_COMPLETABLE_TILES >> 8) as u8, //  2: CMP Map_Completable_Tiles,X   ; loop
    0xF0, 0x1C,                             //  5: BEQ +28 -> yes
    0xCA,                                   //  7: DEX
    0x10, 0xF8,                             //  8: BPL -8
    // --- Map_Removable_Tiles: rocks, locks, forts, the water gap ---
    0xA2, 0x07,                             // 10: LDX #7
    0xDD, MAP_REMOVABLE_TILES as u8,
          (MAP_REMOVABLE_TILES >> 8) as u8, // 12: CMP Map_Removable_Tiles,X      ; loop
    0xF0, 0x12,                             // 15: BEQ +18 -> yes
    0xCA,                                   // 17: DEX
    0x10, 0xF8,                             // 18: BPL -8
    // --- Tile_Attributes_TS0: "enterable" by page and value ---
    0x85, ZP_TILE,                          // 20: STA ZP_TILE
    0x29, 0xC0,                             // 22: AND #$C0
    0x18,                                   // 24: CLC
    0x2A,                                   // 25: ROL A
    0x2A,                                   // 26: ROL A
    0x2A,                                   // 27: ROL A       ; A = tile >> 6
    0xAA,                                   // 28: TAX
    0xA5, ZP_TILE,                          // 29: LDA ZP_TILE
    0xDD, TILE_ATTRIBUTES_TS0 as u8,
          (TILE_ATTRIBUTES_TS0 >> 8) as u8, // 31: CMP Tile_Attributes_TS0,X
    0x60,                                   // 34: RTS         ; carry IS the answer
    0x38,                                   // 35: SEC         ; yes
    0x60,                                   // 36: RTS
];

/// Write one world's stencil — one mask byte per map column, in
/// `Map_Completions`' own bit layout — to `MASK_SCRATCH`.
///
/// `A` is the world index on entry. PRG012 must be at `$A000`; see the hook
/// note above.
///
/// **Two indices advance differently and that is the whole difficulty.** The
/// grid is screen-major, 144 bytes per screen, so a cell is
/// `screen_base + row * 16 + column_in_screen`; `Map_Completions` is indexed by
/// absolute column with a bit per row. Keeping `Y` as `row * 16 + col_in_screen`
/// makes the first free — it never exceeds 143, so one `[ZP_GRID],Y` covers a
/// whole screen and stepping a row is `+16`. The screen base moves by 144 every
/// sixteenth column, and the output index is just a running count.
///
/// **Rows 7 and 8 share bit `$01`.** The row loop runs eight times because
/// `LSR ZP_BIT` reaches zero after the eighth, and row 8 is then handled on its
/// own with `ORA #$01` — the fold [`world_mask`] does with `row.min(7)`.
#[rustfmt::skip]
const MASK_BUILD: [u8; 110] = [
    // --- point ZP_GRID at world A's grid, via Map_Tile_Layouts ---
    0x0A,                                   //  0: ASL A               ; world * 2
    0xAA,                                   //  1: TAX
    0xBD, MAP_TILE_LAYOUTS as u8,
          (MAP_TILE_LAYOUTS >> 8) as u8,    //  2: LDA Map_Tile_Layouts,X
    0x85, ZP_GRID,                          //  5: STA ZP_GRID
    0xBD, (MAP_TILE_LAYOUTS + 1) as u8,
          ((MAP_TILE_LAYOUTS + 1) >> 8) as u8, //  7: LDA Map_Tile_Layouts+1,X
    0x85, ZP_GRID + 1,                      // 10: STA ZP_GRID+1
    0x8A,                                   // 12: TXA
    0x4A,                                   // 13: LSR A               ; world again
    0xAA,                                   // 14: TAX
    0xBD, WORLD_COLS_CPU as u8,
          (WORLD_COLS_CPU >> 8) as u8,      // 15: LDA WorldCols,X
    0x8D, COLS_LEFT as u8, (COLS_LEFT >> 8) as u8, // 18: STA COLS_LEFT
    0xA9, 0x00,                             // 21: LDA #$00
    0x8D, COL_IDX as u8, (COL_IDX >> 8) as u8,     // 23: STA COL_IDX

    // --- one column ---
    0xA9, 0x80,                             // 26: LDA #$80            ; col_loop
    0x85, ZP_BIT,                           // 28: STA ZP_BIT
    0xA9, 0x00,                             // 30: LDA #$00
    0x85, ZP_MASK,                          // 32: STA ZP_MASK
    0xAD, COL_IDX as u8, (COL_IDX >> 8) as u8, // 34: LDA COL_IDX
    0x29, 0x0F,                             // 37: AND #$0F            ; column in screen
    0xA8,                                   // 39: TAY                 ; Y = row 0 offset

    // --- rows 0..7, one bit each ---
    0xB1, ZP_GRID,                          // 40: LDA (ZP_GRID),Y     ; row_loop
    0x20, IS_COMPLETABLE_CPU as u8,
          (IS_COMPLETABLE_CPU >> 8) as u8,  // 42: JSR IsCompletable
    0x90, 0x06,                             // 45: BCC +6
    0xA5, ZP_MASK,                          // 47: LDA ZP_MASK
    0x05, ZP_BIT,                           // 49: ORA ZP_BIT
    0x85, ZP_MASK,                          // 51: STA ZP_MASK
    0x98,                                   // 53: TYA
    0x18,                                   // 54: CLC
    0x69, 0x10,                             // 55: ADC #16             ; next row
    0xA8,                                   // 57: TAY
    0x46, ZP_BIT,                           // 58: LSR ZP_BIT
    0xD0, 0xEA,                             // 60: BNE -22 -> row_loop ; 8 rows, then 0

    // --- row 8, sharing row 7's bit ---
    0xB1, ZP_GRID,                          // 62: LDA (ZP_GRID),Y     ; Y = col + 128
    0x20, IS_COMPLETABLE_CPU as u8,
          (IS_COMPLETABLE_CPU >> 8) as u8,  // 64: JSR IsCompletable
    0x90, 0x06,                             // 67: BCC +6
    0xA5, ZP_MASK,                          // 69: LDA ZP_MASK
    0x09, 0x01,                             // 71: ORA #$01
    0x85, ZP_MASK,                          // 73: STA ZP_MASK

    // --- store it, and move to the next column ---
    0xAE, COL_IDX as u8, (COL_IDX >> 8) as u8, // 75: LDX COL_IDX
    0xA5, ZP_MASK,                          // 78: LDA ZP_MASK
    0x9D, MASK_SCRATCH as u8,
          (MASK_SCRATCH >> 8) as u8,        // 80: STA MASK_SCRATCH,X
    0xEE, COL_IDX as u8, (COL_IDX >> 8) as u8, // 83: INC COL_IDX
    0xAD, COL_IDX as u8, (COL_IDX >> 8) as u8, // 86: LDA COL_IDX
    0x29, 0x0F,                             // 89: AND #$0F
    0xD0, 0x0B,                             // 91: BNE +11 -> next     ; still this screen
    // sixteenth column: the next screen starts 144 bytes on
    0xA5, ZP_GRID,                          // 93: LDA ZP_GRID
    0x18,                                   // 95: CLC
    0x69, 0x90,                             // 96: ADC #144
    0x85, ZP_GRID,                          // 98: STA ZP_GRID
    0x90, 0x02,                             // 100: BCC +2
    0xE6, ZP_GRID + 1,                      // 102: INC ZP_GRID+1
    0xCE, COLS_LEFT as u8, (COLS_LEFT >> 8) as u8, // 104: DEC COLS_LEFT   ; next
    0xD0, 0xAD,                             // 107: BNE -83 -> col_loop
    0x60,                                   // 109: RTS
];

/// Write the console-side routines into PRG010.
///
/// Split out from any hook so the stencil can be exercised — and audited —
/// before anything calls it. Wiring it into `PRG030_84A0` is the next step.
pub(crate) fn apply(rom: &mut Rom) {
    rom.push_tag("completion_bits");
    rom.write_range(FS_MASK_BUILD, &MASK_BUILD);
    rom.write_range(FS_IS_COMPLETABLE, &IS_COMPLETABLE);
    rom.write_range(FS_WORLD_COLS, &WORLD_COLS);
    rom.pop_tag();
}

#[cfg(test)]
mod tests {
    use super::*;

    use mos6502::Variant;
    use mos6502::cpu::CPU;
    use mos6502::instruction::{Instruction, Ricoh2a03};
    use mos6502::memory::{Bus, Memory};

    use crate::randomize::rom_data::asm;

    const ROM_PATH: &str = "roms/Super Mario Bros. 3 (USA) (Rev 1).nes";

    fn vanilla() -> Option<Rom> {
        let bytes = std::fs::read(ROM_PATH).ok()?;
        Some(Rom::from_bytes(&bytes).expect("vanilla ROM parses"))
    }

    fn seeds() -> u64 {
        std::env::var("CENSUS_SEEDS").ok().and_then(|s| s.parse().ok()).unwrap_or(10)
    }

    /// A named option arm, the same shape `completion_bit_census` uses.
    type Arm = (&'static str, fn(&mut crate::Options));

    /// The option arms `completion_bit_census` uses — the ones that move the
    /// *cell count* rather than the routing.
    fn arms() -> [Arm; 3] {
        [
            ("defaults", |_| {}),
            ("rocks+8s-wild", |o| {
                o.more_hammer_rocks = crate::Tri::On;
                o.eights_are_wild = crate::Tri::On;
            }),
            ("all-promotions", |o| {
                o.more_hammer_rocks = crate::Tri::On;
                o.eights_are_wild = crate::Tri::On;
                o.shuffle_spade_games = true;
                o.shuffle_toad_houses = true;
                o.shuffle_hammer_bros = true;
                o.include_beta_stages = true;
            }),
        ]
    }

    /// Count of cells the engine's completion routine can act on — the
    /// predicate `completion_bit_census` measures the RAM budget with.
    fn completable_cells(grid: &Grid) -> usize {
        let mut n = 0;
        for row in 0..grid.rows() {
            for col in 0..grid.cols {
                if is_completion_unsafe(grid.get(row, col)) {
                    n += 1;
                }
            }
        }
        n
    }

    /// One bit per completable cell, and no bit without one.
    ///
    /// The mask folds row 8 onto row 7's `$01`, so it can only agree with a
    /// straight cell count while the builder's `is_row78_conflict` invariant
    /// holds. That makes this an assertion about two things at once, which is
    /// the point: a column carrying a completable cell in *both* row 7 and row
    /// 8 is a map the engine cannot represent, and it would show up here as a
    /// bit shortfall rather than as a mystery on a player's cartridge.
    ///
    /// ```sh
    /// CENSUS_SEEDS=200 cargo test --release --lib bit_per_completable_cell
    /// ```
    /// Where two map cells are fighting over one completion bit.
    ///
    /// Rows 7 and 8 share bit `$01` per column, so a column holding a cell the
    /// engine can act on in *both* rows has one bit for two jobs. Row 7 wins:
    /// `PRG012_A55C` only steps down to row 8 when row 7's tile is not one it
    /// recognises. So a level panel placed at row 8 under a completable row-7
    /// tile would be marked beaten on the tile above it and never on itself.
    ///
    /// `WorldState::row78_barred` is what prevents that, and
    /// `row78_completion_bit_is_never_double_claimed` asserts it on the written
    /// ROM. This is the same invariant seen from the storage side: the mask
    /// folds the pair onto one bit, so a shortfall against the plain cell count
    /// *is* a collision, found without knowing anything about placement.
    /// Cheap, and independent — if the two ever disagree, one of them is wrong.
    ///
    /// A count of zero is not the pass condition. `decor` collisions are two
    /// scenery tiles that merely score as completable — World 6's `$EA` ice
    /// band runs the length of both rows — and nothing ever sets their bit, so
    /// they cost nothing and the builder rightly leaves them alone. **`LIVE`
    /// collisions, with placed content on one side, are the failure**: 47 of
    /// them over 150 builds before the terrain claimant was handled (PR #212),
    /// zero after.
    ///
    /// ```sh
    /// CENSUS_SEEDS=50 cargo test --release --lib row78_collision_census \
    ///     -- --ignored --nocapture
    /// ```
    #[test]
    #[ignore]
    fn row78_collision_census() {
        let Ok(rom_bytes) = std::fs::read(ROM_PATH) else {
            eprintln!("SKIP: requires the ROM");
            return;
        };

        let mut collisions: Vec<(bool, String)> = Vec::new();
        let mut runs = 0usize;
        for (name, arm) in arms() {
            for seed in 0..seeds() {
                let mut options =
                    crate::Options { palettes: false, palette_themed: false, ..Default::default() };
                arm(&mut options);
                let Ok((rom, build)) =
                    crate::randomize_rom_with_overworld_capture(&rom_bytes, seed, &options, None)
                else {
                    continue;
                };
                runs += 1;
                let map = CompletionMap::from_rom(&rom);
                for w in 0..8 {
                    let grid = rom_data::read_tile_grid(&rom, w);
                    // The mask folds the pair onto one bit, so a shortfall
                    // against the cell count is exactly a collision.
                    if map.bits(w) == completable_cells(&grid) {
                        continue;
                    }
                    let placed: std::collections::HashSet<(usize, usize)> = build.worlds[w]
                        .slots
                        .iter()
                        .map(|s| s.pos)
                        .chain(build.worlds[w].locks.iter().map(|l| l.pos))
                        .collect();
                    for col in 0..grid.cols {
                        if !is_completion_unsafe(grid.get(7, col))
                            || !is_completion_unsafe(grid.get(8, col))
                        {
                            continue;
                        }
                        let live = placed.contains(&(7, col)) || placed.contains(&(8, col));
                        collisions.push((
                            live,
                            format!(
                                "{name} seed {seed} W{} col {col}: row7 {:#04X} row8 {:#04X}",
                                w + 1,
                                grid.get(7, col),
                                grid.get(8, col),
                            ),
                        ));
                    }
                }
            }
        }

        let live: Vec<&String> = collisions.iter().filter(|(l, _)| *l).map(|(_, t)| t).collect();
        eprintln!(
            "{} collisions over {runs} builds; {} with placed content",
            collisions.len(),
            live.len(),
        );
        for t in &live {
            eprintln!("  LIVE {t}");
        }
    }

    /// Vanilla's own fortress-FX table names a `(column, bit)` for every lock
    /// and drawbridge in the game. Each one must be a bit this model stores,
    /// or busting that lock would be forgotten on the way out of the world.
    ///
    /// This is the engine's answer rather than the builder's: the entries are
    /// hand-authored ROM data, and the slot-to-world mapping comes from
    /// `FORTRESS_ENTRIES` — the same derivation `world_persist` uses, and the
    /// one a hand-written list got wrong in the mega-map branch.
    #[test]
    fn vanilla_fx_bits_are_owned() {
        let Some(rom) = vanilla() else {
            eprintln!("SKIP: requires the ROM");
            return;
        };
        let map = CompletionMap::from_rom(&rom);

        for (slot, &(world, _)) in rom_data::FORTRESS_ENTRIES.iter().enumerate() {
            let col = rom.read_byte(rom_data::FX_MAP_COMP_IDX + slot * 2) as usize;
            let bit = rom.read_byte(rom_data::FX_MAP_COMP_IDX + slot * 2 + 1);
            let mask = map.mask(world);
            assert!(col < mask.len(), "FX slot {slot}: column {col} is off W{}'s map", world + 1);
            assert!(
                mask[col] & bit != 0,
                "FX slot {slot} (W{}, column {col}, bit {bit:#04X}) owns no completion bit",
                world + 1,
            );
        }
    }

    /// Round-tripping a `Map_Completions` half through a world's plane is the
    /// identity on the bits that world can hold, and drops the rest.
    ///
    /// Every bit is exercised in both states rather than sampled: with the mask
    /// as the alphabet, "all set" and "all clear" plus the alternating patterns
    /// cover every owned bit's two values and every unowned bit's ability to
    /// leak into a neighbour's slot.
    #[test]
    fn pack_round_trips_owned_bits() {
        let Some(rom) = vanilla() else {
            eprintln!("SKIP: requires the ROM");
            return;
        };
        let map = CompletionMap::from_rom(&rom);

        for w in 0..8 {
            let mask = map.mask(w).to_vec();
            // All-ones is the strongest input: every unowned bit is set too, so
            // anything that mistakes an unowned bit for an owned one shifts the
            // whole stream and shows up as a mismatch downstream.
            for fill in [0x00u8, 0xFF, 0xAA, 0x55] {
                let half = [fill; HALF_LEN];
                let plane = map.pack(w, &half);
                assert_eq!(plane.len(), map.plane_bytes(w), "W{}: plane length", w + 1);

                let back = map.unpack(w, &plane);
                for col in 0..HALF_LEN {
                    let owned = mask.get(col).copied().unwrap_or(0);
                    assert_eq!(
                        back[col],
                        half[col] & owned,
                        "W{} column {col}, fill {fill:#04X}: round trip",
                        w + 1,
                    );
                }
            }
        }
    }

    /// The packed region fits the free SRAM the design depends on, with the
    /// margin reported rather than assumed.
    ///
    /// `completion_bit_census` measures the same budget from the cell side;
    /// this measures it from the side that actually allocates, including the
    /// byte alignment each world's slice pays for being addressable on its own.
    #[test]
    fn packed_region_fits_free_sram() {
        let Ok(rom_bytes) = std::fs::read(ROM_PATH) else {
            eprintln!("SKIP: requires the ROM");
            return;
        };

        // The two largest anonymous `.ds` runs in the disassembly, each
        // referenced nowhere but its own declaration, and the pair the
        // `world_persist` POC already proved free at runtime.
        const LARGEST_RUNS: usize = 109 + 105;

        let mut worst = 0usize;
        let mut worst_at = (0u64, "", [0u8; 9]);
        // The stencil is ROM data, not SRAM, but its size decides whether the
        // console re-derives it or reads it — so it is measured here too. Raw
        // is one mask byte per column each world actually has; the alternative
        // is a per-world 8-byte "which columns are non-zero" bitmap plus one
        // byte per non-zero column.
        let mut stencil_raw = 0usize;
        let mut stencil_sparse = 0usize;
        for (name, arm) in arms() {
            for seed in 0..seeds() {
                let mut options =
                    crate::Options { palettes: false, palette_themed: false, ..Default::default() };
                arm(&mut options);
                let Ok((rom, _)) =
                    crate::randomize_rom_with_overworld_capture(&rom_bytes, seed, &options, None)
                else {
                    continue;
                };
                let map = CompletionMap::from_rom(&rom);
                if map.total_bytes() > worst {
                    worst = map.total_bytes();
                    worst_at = (seed, name, map.base_table());
                }
                let raw: usize = (0..8).map(|w| map.mask(w).len()).sum();
                // The presence bitmap is sized per world — World 1's 16
                // columns need two bytes, not World 8's eight.
                let sparse: usize = (0..8)
                    .map(|w| {
                        let m = map.mask(w);
                        m.len().div_ceil(8) + m.iter().filter(|b| **b != 0).count()
                    })
                    .sum();
                stencil_raw = stencil_raw.max(raw);
                stencil_sparse = stencil_sparse.max(sparse);
            }
        }

        let (seed, name, table) = worst_at;
        eprintln!("worst packed region: {worst} bytes (seed {seed}, arm {name})");
        eprintln!("  base table {table:?}  (both planes: 2 x {} bytes)", table[8]);
        eprintln!("  against {LARGEST_RUNS} bytes in the two proven-free SRAM runs");
        eprintln!("stencil, if emitted as ROM data: {stencil_raw} raw, {stencil_sparse} sparse");
        assert!(
            worst <= LARGEST_RUNS,
            "packed completion region needs {worst} bytes, more than the {LARGEST_RUNS} \
             the two proven-free SRAM runs hold",
        );
    }

    // -----------------------------------------------------------------------
    // The console side
    // -----------------------------------------------------------------------

    /// A CPU holding both routines at their real origins, plus the four PRG012
    /// tables they read.
    ///
    /// `mos6502::memory::Memory` is a flat 64K array, so "PRG010 at `$C000`"
    /// and "PRG012 at `$A000`" are just two `set_bytes` calls — the bank
    /// arrangement the hook site guarantees, reproduced literally.
    fn cpu_with_routines(rom: &Rom) -> CPU<Memory, Ricoh2a03> {
        let mut mem = Memory::new();
        mem.set_bytes(MASK_BUILD_CPU, &MASK_BUILD);
        mem.set_bytes(IS_COMPLETABLE_CPU, &IS_COMPLETABLE);
        mem.set_bytes(WORLD_COLS_CPU, &WORLD_COLS);
        // PRG012, whole bank — the tables and every world's grid, at the
        // addresses the routine names.
        let prg012: Vec<u8> =
            (0..0x2000).map(|i| rom.read_byte(rom_data::PRG012_FILE_BASE + i)).collect();
        mem.set_bytes(0xA000, &prg012);
        CPU::new(mem, Ricoh2a03)
    }

    /// Run `MASK_BUILD` for one world and read the stencil back out of RAM.
    fn build_mask_on_cpu(cpu: &mut CPU<Memory, Ricoh2a03>, world: usize) -> Vec<u8> {
        // Poison the scratch: a column the routine never writes stays visible
        // as $AA rather than passing as an accidental zero.
        for i in 0..64u16 {
            cpu.memory.set_byte(MASK_SCRATCH + i, 0xAA);
        }
        cpu.registers.program_counter = MASK_BUILD_CPU;
        cpu.registers.accumulator = world as u8;
        cpu.registers.stack_pointer = mos6502::registers::StackPointer(0xFF);

        // Generous: ~64 columns x 9 rows x ~15 instructions, plus the call
        // overhead. The bound only has to catch a runaway, not be tight.
        for _ in 0..400_000 {
            let op = cpu.memory.get_byte(cpu.registers.program_counter);
            if cpu.registers.program_counter == MASK_BUILD_CPU + MASK_BUILD.len() as u16 - 1
                && matches!(Ricoh2a03::decode(op), Some((Instruction::RTS, _)))
            {
                let cols = WORLD_COLS[world] as u16;
                return (0..cols).map(|i| cpu.memory.get_byte(MASK_SCRATCH + i)).collect();
            }
            cpu.single_step();
        }
        panic!("MASK_BUILD ran away on world {}", world + 1);
    }

    /// **The equivalence test.** The 6502 routine and [`world_mask`] must agree
    /// byte for byte, on maps the randomizer actually produces.
    ///
    /// This is the test the whole packing scheme rests on. The Rust side
    /// decides the bit order and the base table at build time; the 6502 side
    /// has to reach the same stencil on the console from the grid alone, or a
    /// player's progress comes back attached to the wrong cells. Nothing about
    /// that failure is visible in a well-formed-bytes check — the routine would
    /// decode fine and compute the wrong answer.
    ///
    /// The two indices are why: the grid is screen-major, 144 bytes per screen,
    /// while the stencil is indexed by absolute column. An off-by-one in either
    /// survives `asm::check` and shows up only here.
    ///
    /// ```sh
    /// CENSUS_SEEDS=100 cargo test --release --lib mask_build_matches_rust
    /// ```
    #[test]
    fn mask_build_matches_rust() {
        let Ok(rom_bytes) = std::fs::read(ROM_PATH) else {
            eprintln!("SKIP: requires the ROM");
            return;
        };

        // Vanilla first: if the routine is wrong, this says so without a
        // randomizer run in the way.
        let van = Rom::from_bytes(&rom_bytes).expect("vanilla ROM parses");
        let map = CompletionMap::from_rom(&van);
        let mut cpu = cpu_with_routines(&van);
        for w in 0..8 {
            assert_eq!(
                build_mask_on_cpu(&mut cpu, w),
                map.mask(w),
                "vanilla W{}: 6502 stencil disagrees with Rust",
                w + 1,
            );
        }

        for (name, arm) in arms() {
            for seed in 0..seeds() {
                let mut options =
                    crate::Options { palettes: false, palette_themed: false, ..Default::default() };
                arm(&mut options);
                let Ok((rom, _)) =
                    crate::randomize_rom_with_overworld_capture(&rom_bytes, seed, &options, None)
                else {
                    continue;
                };
                let map = CompletionMap::from_rom(&rom);
                let mut cpu = cpu_with_routines(&rom);
                for w in 0..8 {
                    assert_eq!(
                        build_mask_on_cpu(&mut cpu, w),
                        map.mask(w),
                        "{name} seed {seed} W{}: 6502 stencil disagrees with Rust",
                        w + 1,
                    );
                }
            }
        }
    }

    /// Every column count the routine bounds its walk with must match the grid
    /// it is walking, or it reads off the end of one world into the next.
    #[test]
    fn world_cols_matches_the_grid_table() {
        for (w, &cols) in WORLD_COLS.iter().enumerate() {
            assert_eq!(
                cols as usize,
                rom_data::MAP_TILE_GRIDS[w].columns,
                "W{}: column count",
                w + 1,
            );
        }
    }

    #[test]
    fn routines_are_well_formed() {
        asm::check(&IS_COMPLETABLE)
            .allocation(FS_IS_COMPLETABLE)
            .origin(IS_COMPLETABLE_CPU)
            .assert_ok();
        asm::check(&MASK_BUILD).allocation(FS_MASK_BUILD).origin(MASK_BUILD_CPU).assert_ok();
    }
}
