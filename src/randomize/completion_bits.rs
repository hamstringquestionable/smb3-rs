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

use super::rom_data::{
    FS_COMPLETION_BASES, FS_IS_COMPLETABLE, FS_MASK_BUILD, FS_PACK_PLANE, FS_PACK_WORLD,
    FS_SWAP_AT_RELOAD, FS_UNPACK_PLANE, FS_UNPACK_WORLD, FS_WIPE_REPLACEMENT, FS_WORLD_COLS,
};

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
const PACK_PLANE_CPU: u16 = prg010_cpu(FS_PACK_PLANE);
const UNPACK_PLANE_CPU: u16 = prg010_cpu(FS_UNPACK_PLANE);
const PACK_WORLD_CPU: u16 = prg010_cpu(FS_PACK_WORLD);
const UNPACK_WORLD_CPU: u16 = prg010_cpu(FS_UNPACK_WORLD);
const WIPE_REPLACEMENT_CPU: u16 = prg010_cpu(FS_WIPE_REPLACEMENT);
const SWAP_AT_RELOAD_CPU: u16 = prg010_cpu(FS_SWAP_AT_RELOAD);
const BASES_CPU: u16 = prg010_cpu(FS_COMPLETION_BASES);

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

// The transfer routines run after the stencil is built and reuse the same five
// bytes for their own purposes. Named separately because they mean different
// things, aliased deliberately because there is no sixth safe byte.

/// `Temp_Var1`/`Temp_Var2` — the `Map_Completions` half being transferred.
const ZP_SRC: u8 = 0x00;
/// `Temp_Var5` — the byte of packed bits in flight.
const ZP_ACC: u8 = 0x04;

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

/// Where a world's packed planes live: Mario's at `PACKED`, the mirror's
/// [`PLANE_RESERVE`] bytes further on, each world at its own base-table offset.
///
/// `$7997-$79FF` is the second of the two SRAM runs the POC proved free at
/// runtime. One absolute base covers both planes because the mirror is reached
/// by a starting index of `base + PLANE_RESERVE`, never by a second address.
const PACKED: u16 = 0x7997;

/// Bytes reserved for one plane — all eight worlds' slices end to end.
///
/// The measured worst case over 150 builds is 42; the margin is deliberate,
/// since a seed that needed one byte more would silently write into the mirror.
/// `packed_planes_fit_their_reserve` holds the line.
pub(crate) const PLANE_RESERVE: usize = 48;

/// Bits still unread in, or unwritten to, the byte being transferred.
const BITCNT: u16 = 0x7AB5;

/// The world whose completions are currently live in `$7D00`.
///
/// `World_Num` cannot answer this at the hook: both of vanilla's paths into
/// `PRG030_84A0` set it to the *destination* first — `INC World_Num` on the
/// airship, `LDA Map_Warp_PrevWorld / STA World_Num` in the warp zone — so by
/// the time the map re-initialises, the world being left has no name. One byte
/// remembers it, and the raw swap the POC used needed none only because with
/// two worlds "the other one" is unambiguous.
const LIVE_WORLD: u16 = 0x7ABC;

/// Set by the routines that transition between worlds; cleared once acted on.
///
/// The `Map_Completions` wipe this replaces was also vanilla's new-game reset,
/// so removing it means a fresh game would otherwise inherit whatever SRAM
/// held. The title screen falls *through* into `PRG030_84A0` while every
/// transition jumps to it, and the jumps this POC uses are all our own code, so
/// they raise the flag on the way past — see `world_persist`'s `WORLD_JUMP_CHECK`
/// and `PORTAL_EXIT`. Absent the flag, the replacement resets instead of packing.
///
/// **Vanilla's own two jumps are not flagged.** The airship-cleared path and the
/// warp zone would each be read as a new game and reset every world. Neither
/// occurs in a maze, so the POC does not need them; closing the gap means a
/// trampoline in PRG030, whose airship-side site `world_order` already claims.
pub(crate) const TRANSITION_FLAG: u16 = 0x7ABD;

/// Compress one `Map_Completions` half into a world's plane.
///
/// `A` is the starting byte index — `base_table[world]` for Mario's half,
/// plus [`PLANE_RESERVE`] for the mirror. `Temp_Var1`/`2` point at the half
/// being read, and [`MASK_SCRATCH`] and `COL_IDX` must be what [`MASK_BUILD`]
/// left for the same world.
///
/// The stencil says which of the 512 bits can ever be set; this takes exactly
/// those, most significant first, and packs them end to end. A column whose
/// mask is zero — half of them — skips its inner loop entirely.
///
/// The tail byte is left-aligned by shifting it up by the count still
/// outstanding, so [`UNPACK_PLANE`] can read it back with the same
/// most-significant-first walk and never has to know where the stream stopped.
#[rustfmt::skip]
const PACK_PLANE: [u8; 89] = [
    0xAA,                                   //  0: TAX             ; plane byte index
    0xA9, 0x00,                             //  1: LDA #$00
    0x85, ZP_ACC,                           //  3: STA ZP_ACC
    0xA9, 0x08,                             //  5: LDA #$08
    0x8D, BITCNT as u8, (BITCNT >> 8) as u8, //  7: STA BITCNT
    0xA0, 0x00,                             // 10: LDY #$00        ; column

    0xB9, MASK_SCRATCH as u8,
          (MASK_SCRATCH >> 8) as u8,        // 12: LDA MASK_SCRATCH,Y   ; col_loop
    0xF0, 0x2E,                             // 15: BEQ +46 -> nextcol   ; owns nothing
    0x85, ZP_MASK,                          // 17: STA ZP_MASK
    0xA9, 0x80,                             // 19: LDA #$80
    0x85, ZP_BIT,                           // 21: STA ZP_BIT

    0xA5, ZP_MASK,                          // 23: LDA ZP_MASK          ; bit_loop
    0x25, ZP_BIT,                           // 25: AND ZP_BIT
    0xF0, 0x1E,                             // 27: BEQ +30 -> nobit     ; not owned
    0x18,                                   // 29: CLC
    0xB1, ZP_SRC,                           // 30: LDA (ZP_SRC),Y
    0x25, ZP_BIT,                           // 32: AND ZP_BIT
    0xF0, 0x01,                             // 34: BEQ +1               ; leave carry clear
    0x38,                                   // 36: SEC
    0x26, ZP_ACC,                           // 37: ROL ZP_ACC           ; shift the bit in
    0xCE, BITCNT as u8, (BITCNT >> 8) as u8, // 39: DEC BITCNT
    0xD0, 0x0F,                             // 42: BNE +15 -> nobit     ; byte not full
    0xA5, ZP_ACC,                           // 44: LDA ZP_ACC
    0x9D, PACKED as u8, (PACKED >> 8) as u8, // 46: STA PACKED,X
    0xE8,                                   // 49: INX
    0xA9, 0x08,                             // 50: LDA #$08
    0x8D, BITCNT as u8, (BITCNT >> 8) as u8, // 52: STA BITCNT
    0xA9, 0x00,                             // 55: LDA #$00
    0x85, ZP_ACC,                           // 57: STA ZP_ACC

    0x46, ZP_BIT,                           // 59: LSR ZP_BIT           ; nobit
    0xD0, 0xD8,                             // 61: BNE -40 -> bit_loop

    0xC8,                                   // 63: INY                  ; nextcol
    0xCC, COL_IDX as u8, (COL_IDX >> 8) as u8, // 64: CPY COL_IDX
    0xD0, 0xC7,                             // 67: BNE -57 -> col_loop

    // --- the tail: left-align whatever is outstanding and store it ---
    0xAD, BITCNT as u8, (BITCNT >> 8) as u8, // 69: LDA BITCNT
    0xC9, 0x08,                             // 72: CMP #$08
    0xF0, 0x0C,                             // 74: BEQ +12 -> done      ; nothing pending
    0x06, ZP_ACC,                           // 76: ASL ZP_ACC           ; flush_loop
    0xCE, BITCNT as u8, (BITCNT >> 8) as u8, // 78: DEC BITCNT
    0xD0, 0xF9,                             // 81: BNE -7 -> flush_loop
    0xA5, ZP_ACC,                           // 83: LDA ZP_ACC
    0x9D, PACKED as u8, (PACKED >> 8) as u8, // 85: STA PACKED,X
    0x60,                                   // 88: RTS                  ; done
];

/// Expand a world's plane back into one `Map_Completions` half.
///
/// The mirror of [`PACK_PLANE`], with the same entry conditions.
///
/// **Every column is written, including the ones that own nothing.** The half
/// still holds the world you just left, so a column skipped rather than cleared
/// would carry its bits across — which is the failure the whole scheme exists
/// to avoid, and it would look like progress appearing in a world you had never
/// played.
///
/// `BITCNT` starts at zero so the first `DEC` goes negative and forces the
/// first byte to be fetched; after that it counts 7 down to 0 across each
/// byte's eight bits.
#[rustfmt::skip]
const UNPACK_PLANE: [u8; 66] = [
    0xAA,                                   //  0: TAX             ; plane byte index
    0xA9, 0x00,                             //  1: LDA #$00
    0x8D, BITCNT as u8, (BITCNT >> 8) as u8, //  3: STA BITCNT      ; forces a fetch
    0xA0, 0x00,                             //  6: LDY #$00        ; column

    0xA9, 0x00,                             //  8: LDA #$00        ; col_loop
    0x91, ZP_SRC,                           // 10: STA (ZP_SRC),Y  ; clear it first
    0xB9, MASK_SCRATCH as u8,
          (MASK_SCRATCH >> 8) as u8,        // 12: LDA MASK_SCRATCH,Y
    0xF0, 0x2A,                             // 15: BEQ +42 -> nextcol
    0x85, ZP_MASK,                          // 17: STA ZP_MASK
    0xA9, 0x80,                             // 19: LDA #$80
    0x85, ZP_BIT,                           // 21: STA ZP_BIT

    0xA5, ZP_MASK,                          // 23: LDA ZP_MASK          ; bit_loop
    0x25, ZP_BIT,                           // 25: AND ZP_BIT
    0xF0, 0x1A,                             // 27: BEQ +26 -> nobit
    0xCE, BITCNT as u8, (BITCNT >> 8) as u8, // 29: DEC BITCNT
    0x10, 0x0B,                             // 32: BPL +11 -> have      ; still loaded
    0xBD, PACKED as u8, (PACKED >> 8) as u8, // 34: LDA PACKED,X
    0x85, ZP_ACC,                           // 37: STA ZP_ACC
    0xE8,                                   // 39: INX
    0xA9, 0x07,                             // 40: LDA #$07
    0x8D, BITCNT as u8, (BITCNT >> 8) as u8, // 42: STA BITCNT
    0x06, ZP_ACC,                           // 45: ASL ZP_ACC           ; have
    0x90, 0x06,                             // 47: BCC +6 -> nobit
    0xB1, ZP_SRC,                           // 49: LDA (ZP_SRC),Y
    0x05, ZP_BIT,                           // 51: ORA ZP_BIT
    0x91, ZP_SRC,                           // 53: STA (ZP_SRC),Y

    0x46, ZP_BIT,                           // 55: LSR ZP_BIT           ; nobit
    0xD0, 0xDC,                             // 57: BNE -36 -> bit_loop

    0xC8,                                   // 59: INY                  ; nextcol
    0xCC, COL_IDX as u8, (COL_IDX >> 8) as u8, // 60: CPY COL_IDX
    0xD0, 0xC7,                             // 63: BNE -57 -> col_loop
    0x60,                                   // 65: RTS
];

// --- Engine symbols outside PRG012 ---

/// `World_Num`, 0-based.
const WORLD_NUM: u16 = 0x0727;
/// `PAGE_A000` — the MMC3 page latched into `$A000` by the next
/// `PRGROM_Change_A000`.
const PAGE_A000: u16 = 0x0720;
/// `PRGROM_Change_A000`, in the always-mapped PRG031.
const PRGROM_CHANGE_A000: u16 = 0xFFC2;
/// `Map_Reload_with_Completions` (PRG012) — the call this displaces.
const MAP_RELOAD: u16 = 0xA45D;

/// Bytes of packed state in total: both planes, back to back.
const PACKED_LEN: usize = 2 * PLANE_RESERVE;

/// Transfer one world's *pair* of planes, Mario's and the mirror's.
///
/// `A` is the world index. Builds the stencil first, then runs the plane
/// routine twice — `$7D00` into `base_table[world]`, then `$7D40` into
/// `base_table[world] + PLANE_RESERVE`. Only the low byte of the source
/// pointer differs between the two, which is why the pair costs so little.
///
/// The base is stacked across the first call because the plane routine uses
/// `X` as its own output cursor.
macro_rules! xfer_world {
    ($plane:expr) => {
        [
            0x48, //  0: PHA           ; keep the world
            0x20,
            MASK_BUILD_CPU as u8,
            (MASK_BUILD_CPU >> 8) as u8, //  1: JSR MASK_BUILD
            0x68,                        //  4: PLA
            0xAA,                        //  5: TAX
            0xA9,
            0x7D, //  6: LDA #$7D
            0x85,
            ZP_SRC + 1, //  8: STA ZP_SRC+1
            0xA9,
            0x00, // 10: LDA #$00
            0x85,
            ZP_SRC, // 12: STA ZP_SRC    ; -> $7D00
            0xBD,
            BASES_CPU as u8,
            (BASES_CPU >> 8) as u8, // 14: LDA Bases,X
            0x48,                   // 17: PHA
            0x20,
            $plane as u8,
            ($plane >> 8) as u8, // 18: JSR <plane>
            0xA9,
            0x40, // 21: LDA #$40
            0x85,
            ZP_SRC, // 23: STA ZP_SRC    ; -> $7D40
            0x68,   // 25: PLA
            0x18,   // 26: CLC
            0x69,
            PLANE_RESERVE as u8, // 27: ADC #PLANE_RESERVE
            0x20,
            $plane as u8,
            ($plane >> 8) as u8, // 29: JSR <plane>
            0x60,                // 32: RTS
        ]
    };
}

/// Compress a world out of `$7D00` into its slice. See [`xfer_world!`].
const PACK_WORLD: [u8; 33] = xfer_world!(PACK_PLANE_CPU);
/// Expand a world's slice back into `$7D00`. See [`xfer_world!`].
const UNPACK_WORLD: [u8; 33] = xfer_world!(UNPACK_PLANE_CPU);

/// What stands where `PRG030_84A0`'s `Map_Completions` wipe used to.
///
/// **It must never destroy `Map_Completions`. That is the whole rule, and
/// breaking it is what the first two playtests failed on.**
///
/// `PRG030_84A0` is entered far more often than a world change. Vanilla wipes
/// the array every time and can afford to, because vanilla only ever moves
/// forward through worlds — a wiped completion array is never read back. A maze
/// returns to worlds, so every one of those entries has to leave the array
/// alone.
///
/// The two-world POC got this right by construction: it replaced the wipe with
/// a *swap*, which cannot lose a bit down any path. This replaced it with a
/// *branch*, and the arm taken on every non-transition entry was destructive —
/// first by zeroing the array outright, then by parking [`LIVE_WORLD`] out of
/// range so [`SWAP_AT_RELOAD`] expanded a freshly-zeroed plane over it. Both
/// erased the level you had just beaten, on the walk back to the map.
///
/// So: pack the outgoing world when [`TRANSITION_FLAG`] says a jump is under
/// way, and otherwise **return without touching anything**.
///
/// PRG012 has to be banked and unbanked around the pack because the stencil is
/// derived from the map grid, and at this point in the init `$A000` still holds
/// PRG011 for `Map_Init`'s benefit. The restore is a tail `JMP` into
/// `PRGROM_Change_A000`, which returns for us.
///
/// **Knowingly unhandled, exactly as in the POC:** a cold boot with dirty SRAM,
/// and a game over into a new game, both inherit whatever the store held. The
/// POC listed the same gap. Closing it needs a signal that means "new game",
/// and "the transition flag is clear" is emphatically not that signal.
#[rustfmt::skip]
const WIPE_REPLACEMENT: [u8; 33] = [
    0xAD, TRANSITION_FLAG as u8,
          (TRANSITION_FLAG >> 8) as u8,             //  0: LDA TRANSITION_FLAG
    0xF0, 0x1B,                                     //  3: BEQ +27 -> leave it alone

    0xA9, 0x00,                                     //  5: LDA #$00
    0x8D, TRANSITION_FLAG as u8,
          (TRANSITION_FLAG >> 8) as u8,             //  7: STA TRANSITION_FLAG
    0xA9, 0x0C,                                     // 10: LDA #12
    0x8D, PAGE_A000 as u8, (PAGE_A000 >> 8) as u8,  // 12: STA PAGE_A000
    0x20, PRGROM_CHANGE_A000 as u8,
          (PRGROM_CHANGE_A000 >> 8) as u8,          // 15: JSR PRGROM_Change_A000
    0xAD, LIVE_WORLD as u8, (LIVE_WORLD >> 8) as u8, // 18: LDA LIVE_WORLD
    0x20, PACK_WORLD_CPU as u8,
          (PACK_WORLD_CPU >> 8) as u8,              // 21: JSR PACK_WORLD
    0xA9, 0x0B,                                     // 24: LDA #11
    0x8D, PAGE_A000 as u8, (PAGE_A000 >> 8) as u8,  // 26: STA PAGE_A000
    0x4C, PRGROM_CHANGE_A000 as u8,
          (PRGROM_CHANGE_A000 >> 8) as u8,          // 29: JMP PRGROM_Change_A000  ; tail call

    0x60,                                           // 32: RTS   ; leave it alone
];

/// Expand the world being entered — **but only when it changed** — then let the
/// engine draw it.
///
/// Replaces `JSR Map_Reload_with_Completions` at `$85BB`. That call is the only
/// point in the map init where PRG012 (the grid and the tile tables), PRG010
/// (this code) and a settled `World_Num` are all available at once.
///
/// **The guard is not an optimisation, it is the whole correctness of this
/// hook.** `PRG030_84A0` has a second entry at `$84D7` that skips the wipe and
/// runs everything after it, and the map loop jumps there on every turn end —
/// entering a level, returning from one, losing a life. Vanilla depends on that
/// to *keep* completions. Unpacking unconditionally puts the slice saved when
/// the world was entered back over `$7D00`, so a level beaten since then is
/// erased on the walk back to the map. This was the first playtest failure.
///
/// [`WIPE_REPLACEMENT`] leaves `LIVE_WORLD` naming the world it just packed, so
/// a real transition is exactly `World_Num != LIVE_WORLD`; a redraw is exactly
/// equality.
#[rustfmt::skip]
const SWAP_AT_RELOAD: [u8; 18] = [
    0xAD, WORLD_NUM as u8, (WORLD_NUM >> 8) as u8,  //  0: LDA World_Num
    0xCD, LIVE_WORLD as u8, (LIVE_WORLD >> 8) as u8, //  3: CMP LIVE_WORLD
    0xF0, 0x06,                                     //  6: BEQ +6 -> reload   ; a redraw
    0x8D, LIVE_WORLD as u8, (LIVE_WORLD >> 8) as u8, //  8: STA LIVE_WORLD    ; A survives
    0x20, UNPACK_WORLD_CPU as u8,
          (UNPACK_WORLD_CPU >> 8) as u8,            // 11: JSR UNPACK_WORLD
    0x20, MAP_RELOAD as u8, (MAP_RELOAD >> 8) as u8, // 14: JSR Map_Reload...  ; reload
    0x60,                                           // 17: RTS
];

/// The `Map_Completions` wipe in `PRG030_84A0`: CPU `$84CD`, ten bytes, three
/// whole instructions, nothing branching into the middle.
const WIPE_OFFSET: usize = 0x3C4DD;
const WIPE_LEN: usize = 10;

/// `JSR Map_Reload_with_Completions` in `PRG030_84A0`: CPU `$85BB`, three bytes.
const RELOAD_CALL_OFFSET: usize = 0x3C5CB;

/// Vanilla bytes at the two hook sites, for the hook checks.
#[cfg(test)]
#[rustfmt::skip]
const WIPE_VANILLA: [u8; WIPE_LEN] = [
    0xA0, 0x7F,             // LDY #$7F
    0xA9, 0x00,             // LDA #$00
    0x99, 0x00, 0x7D,       // STA Map_Completions,Y
    0x88,                   // DEY
    0x10, 0xFA,             // BPL -6
];
#[cfg(test)]
const RELOAD_CALL_VANILLA: [u8; 3] = [0x20, 0x5D, 0xA4];

/// Install packed per-world completions: the routines, the seed's base table,
/// and the two hooks in `PRG030_84A0`.
///
/// **Run this last.** The base table is derived from the map grids as they
/// stand in the ROM, so anything that still means to move a map tile has to
/// have moved it already.
///
/// The wipe site is left with seven bytes of `NOP` after the call. That is
/// wasteful by this project's standards and deliberate here: `world_persist`
/// writes its arrival restore into three of them, and a shipped version would
/// restructure the surrounding init rather than pad it.
pub(crate) fn apply(rom: &mut Rom) {
    let bases = CompletionMap::from_rom(rom).base_table();

    rom.push_tag("completion_bits");
    rom.write_range(FS_MASK_BUILD, &MASK_BUILD);
    rom.write_range(FS_IS_COMPLETABLE, &IS_COMPLETABLE);
    rom.write_range(FS_WORLD_COLS, &WORLD_COLS);
    rom.write_range(FS_PACK_PLANE, &PACK_PLANE);
    rom.write_range(FS_UNPACK_PLANE, &UNPACK_PLANE);
    rom.write_range(FS_PACK_WORLD, &PACK_WORLD);
    rom.write_range(FS_UNPACK_WORLD, &UNPACK_WORLD);
    rom.write_range(FS_COMPLETION_BASES, &bases);

    rom.write_range(FS_WIPE_REPLACEMENT, &WIPE_REPLACEMENT);
    rom.write_range(FS_SWAP_AT_RELOAD, &SWAP_AT_RELOAD);

    // Hook 1: the wipe becomes a call to the replacement.
    let mut wipe = [0xEA_u8; WIPE_LEN];
    wipe[0] = 0x20; // JSR
    wipe[1] = WIPE_REPLACEMENT_CPU as u8;
    wipe[2] = (WIPE_REPLACEMENT_CPU >> 8) as u8;
    rom.write_range(WIPE_OFFSET, &wipe);

    // Hook 2: the reload call now expands the world first.
    rom.write_range(
        RELOAD_CALL_OFFSET,
        &[0x20, SWAP_AT_RELOAD_CPU as u8, (SWAP_AT_RELOAD_CPU >> 8) as u8],
    );

    rom.pop_tag();
}

#[cfg(test)]
mod tests {
    use super::*;

    use mos6502::cpu::CPU;
    use mos6502::instruction::Ricoh2a03;
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
        mem.set_bytes(PACK_PLANE_CPU, &PACK_PLANE);
        mem.set_bytes(UNPACK_PLANE_CPU, &UNPACK_PLANE);
        mem.set_bytes(PACK_WORLD_CPU, &PACK_WORLD);
        mem.set_bytes(UNPACK_WORLD_CPU, &UNPACK_WORLD);
        mem.set_bytes(BASES_CPU, &CompletionMap::from_rom(rom).base_table());
        mem.set_bytes(WIPE_REPLACEMENT_CPU, &WIPE_REPLACEMENT);
        mem.set_bytes(SWAP_AT_RELOAD_CPU, &SWAP_AT_RELOAD);
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
        cpu.registers.accumulator = world as u8;
        call_routine(cpu, MASK_BUILD_CPU, "MASK_BUILD");
        let cols = WORLD_COLS[world] as u16;
        (0..cols).map(|i| cpu.memory.get_byte(MASK_SCRATCH + i)).collect()
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
        asm::check(&PACK_PLANE).allocation(FS_PACK_PLANE).origin(PACK_PLANE_CPU).assert_ok();
        asm::check(&UNPACK_PLANE).allocation(FS_UNPACK_PLANE).origin(UNPACK_PLANE_CPU).assert_ok();
        asm::check(&PACK_WORLD).allocation(FS_PACK_WORLD).origin(PACK_WORLD_CPU).assert_ok();
        asm::check(&UNPACK_WORLD).allocation(FS_UNPACK_WORLD).origin(UNPACK_WORLD_CPU).assert_ok();
        asm::check(&SWAP_AT_RELOAD)
            .allocation(FS_SWAP_AT_RELOAD)
            .origin(SWAP_AT_RELOAD_CPU)
            .assert_ok();

        asm::check(&WIPE_REPLACEMENT)
            .allocation(FS_WIPE_REPLACEMENT)
            .origin(WIPE_REPLACEMENT_CPU)
            .assert_ok();
    }
    /// Address the harness treats as "the routine returned".
    ///
    /// Nothing is mapped there; the run stops the moment `PC` reaches it.
    const SENTINEL: u16 = 0x0F00;

    /// Call a routine the way the engine would, and stop when it returns.
    ///
    /// A return address is pushed so *any* exit works — plain `RTS`, or the
    /// tail `JMP PRGROM_Change_A000` the wipe replacement ends with. Watching
    /// for an `RTS` at a known offset instead would silently miss the tail
    /// call, which is exactly the shape that hid the first playtest bug.
    fn call_routine(cpu: &mut CPU<Memory, Ricoh2a03>, entry: u16, what: &str) {
        let ret = SENTINEL.wrapping_sub(1);
        cpu.memory.set_byte(0x01FF, (ret >> 8) as u8);
        cpu.memory.set_byte(0x01FE, ret as u8);
        cpu.registers.stack_pointer = mos6502::registers::StackPointer(0xFD);
        cpu.registers.program_counter = entry;
        for _ in 0..400_000 {
            if cpu.registers.program_counter == SENTINEL {
                return;
            }
            cpu.single_step();
        }
        panic!("{what} ran away");
    }

    /// A deterministic filler for a `Map_Completions` half.
    ///
    /// Not `rand`: the point is that a failure is reproducible from the seed
    /// and the world alone, without a captured array to compare against.
    fn fill_half(seed: u64, world: usize, fill: u8) -> [u8; HALF_LEN] {
        let mut half = [0u8; HALF_LEN];
        let mut x = seed.wrapping_mul(0x9E37_79B9).wrapping_add(world as u64 + 1);
        for (i, b) in half.iter_mut().enumerate() {
            x = x.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            *b = match fill {
                0 => 0x00,
                1 => 0xFF,
                2 => (x >> 33) as u8,
                _ => {
                    if i % 2 == 0 {
                        0xAA
                    } else {
                        0x55
                    }
                }
            };
        }
        half
    }

    /// Set a world up on the CPU exactly as the hook will: build the stencil,
    /// then load the half to be transferred.
    fn stage_world(cpu: &mut CPU<Memory, Ricoh2a03>, world: usize, half: &[u8; HALF_LEN]) {
        build_mask_on_cpu(cpu, world);
        for (i, &b) in half.iter().enumerate() {
            cpu.memory.set_byte(0x7D00 + i as u16, b);
        }
        cpu.memory.set_byte(ZP_SRC as u16, 0x00);
        cpu.memory.set_byte(ZP_SRC as u16 + 1, 0x7D);
    }

    /// `MASK_BUILD` must leave `COL_IDX` holding the world's column count —
    /// the transfer routines bound their walk with it rather than reloading.
    #[test]
    fn mask_build_leaves_the_column_count() {
        let Some(rom) = vanilla() else {
            eprintln!("SKIP: requires the ROM");
            return;
        };
        let mut cpu = cpu_with_routines(&rom);
        for (w, &cols) in WORLD_COLS.iter().enumerate() {
            build_mask_on_cpu(&mut cpu, w);
            assert_eq!(cpu.memory.get_byte(COL_IDX), cols, "W{}: COL_IDX after MASK_BUILD", w + 1,);
        }
    }

    /// **`PACK_PLANE` on the console must agree with [`CompletionMap::pack`].**
    ///
    /// Run over the whole pipeline the hook will run — stencil, then compress —
    /// on maps the randomizer produces, with four fills including all-ones. The
    /// all-ones case is the one that catches a bit-order slip: every unowned bit
    /// is set too, so mistaking one for owned shifts the entire stream.
    #[test]
    fn pack_plane_matches_rust() {
        let Ok(rom_bytes) = std::fs::read(ROM_PATH) else {
            eprintln!("SKIP: requires the ROM");
            return;
        };
        for seed in 0..seeds() {
            let options =
                crate::Options { palettes: false, palette_themed: false, ..Default::default() };
            let Ok((rom, _)) =
                crate::randomize_rom_with_overworld_capture(&rom_bytes, seed, &options, None)
            else {
                continue;
            };
            let map = CompletionMap::from_rom(&rom);
            let mut cpu = cpu_with_routines(&rom);
            for w in 0..8 {
                for fill in 0..4u8 {
                    let half = fill_half(seed, w, fill);
                    stage_world(&mut cpu, w, &half);
                    let base = map.base(w);
                    for i in 0..PLANE_RESERVE {
                        cpu.memory.set_byte(PACKED + i as u16, 0xAA); // poison
                    }
                    cpu.registers.accumulator = base as u8;
                    call_routine(&mut cpu, PACK_PLANE_CPU, "PACK_PLANE");
                    let got: Vec<u8> = (0..map.plane_bytes(w))
                        .map(|i| cpu.memory.get_byte(PACKED + (base + i) as u16))
                        .collect();
                    assert_eq!(
                        got,
                        map.pack(w, &half),
                        "seed {seed} W{} fill {fill}: 6502 pack disagrees",
                        w + 1,
                    );
                }
            }
        }
    }

    /// **`UNPACK_PLANE` must undo it, and must clear what it does not own.**
    ///
    /// The half is pre-loaded with the *previous* world's data rather than
    /// zeroes, which is the real situation once the wipe is gone: a column the
    /// routine skips instead of clearing carries progress into a world it never
    /// belonged to.
    #[test]
    fn unpack_plane_matches_rust() {
        let Ok(rom_bytes) = std::fs::read(ROM_PATH) else {
            eprintln!("SKIP: requires the ROM");
            return;
        };
        for seed in 0..seeds() {
            let options =
                crate::Options { palettes: false, palette_themed: false, ..Default::default() };
            let Ok((rom, _)) =
                crate::randomize_rom_with_overworld_capture(&rom_bytes, seed, &options, None)
            else {
                continue;
            };
            let map = CompletionMap::from_rom(&rom);
            let mut cpu = cpu_with_routines(&rom);
            for w in 0..8 {
                for fill in 0..4u8 {
                    let half = fill_half(seed, w, fill);
                    let plane = map.pack(w, &half);
                    let base = map.base(w);
                    for (i, &b) in plane.iter().enumerate() {
                        cpu.memory.set_byte(PACKED + (base + i) as u16, b);
                    }
                    // Stale data from the world just left.
                    let stale = fill_half(seed ^ 0xFFFF, w, 1);
                    stage_world(&mut cpu, w, &stale);
                    cpu.registers.accumulator = base as u8;
                    call_routine(&mut cpu, UNPACK_PLANE_CPU, "UNPACK_PLANE");
                    let want = map.unpack(w, &plane);
                    let got: Vec<u8> = (0..WORLD_COLS[w] as u16)
                        .map(|i| cpu.memory.get_byte(0x7D00 + i))
                        .collect();
                    assert_eq!(
                        got,
                        want[..WORLD_COLS[w] as usize],
                        "seed {seed} W{} fill {fill}: 6502 unpack disagrees",
                        w + 1,
                    );
                }
            }
        }
    }

    /// No seed may need more than the plane reserve, or a world's Mario slice
    /// would run into the mirror region and take the other plane's bits with it.
    #[test]
    fn packed_planes_fit_their_reserve() {
        let Ok(rom_bytes) = std::fs::read(ROM_PATH) else {
            eprintln!("SKIP: requires the ROM");
            return;
        };
        let mut worst = 0usize;
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
                let need = CompletionMap::from_rom(&rom).mirror_offset();
                assert!(
                    need <= PLANE_RESERVE,
                    "{name} seed {seed}: a plane needs {need} bytes, reserve is {PLANE_RESERVE}",
                );
                worst = worst.max(need);
            }
        }
        eprintln!("worst plane: {worst} of {PLANE_RESERVE} reserved");
    }
    /// **The whole storage layer, in and out, on the console.**
    ///
    /// `PACK_WORLD` then `UNPACK_WORLD` for the same world has to be the
    /// identity on every bit the stencil owns, for both halves at once — which
    /// is the only place the mirror's `+PLANE_RESERVE` offset is exercised.
    /// A pack that wrote the mirror over Mario's slice would still round-trip
    /// each half on its own; it fails here.
    #[test]
    fn world_round_trips_on_the_cpu() {
        let Ok(rom_bytes) = std::fs::read(ROM_PATH) else {
            eprintln!("SKIP: requires the ROM");
            return;
        };
        for seed in 0..seeds() {
            let options =
                crate::Options { palettes: false, palette_themed: false, ..Default::default() };
            let Ok((rom, _)) =
                crate::randomize_rom_with_overworld_capture(&rom_bytes, seed, &options, None)
            else {
                continue;
            };
            let map = CompletionMap::from_rom(&rom);
            let mut cpu = cpu_with_routines(&rom);
            for w in 0..8 {
                let mario = fill_half(seed, w, 2);
                let mirror = fill_half(seed ^ 0xABCD, w, 2);
                for (i, &b) in mario.iter().enumerate() {
                    cpu.memory.set_byte(0x7D00 + i as u16, b);
                }
                for (i, &b) in mirror.iter().enumerate() {
                    cpu.memory.set_byte(0x7D40 + i as u16, b);
                }
                // Poison the whole region: a pack that writes outside this
                // world's two slices is invisible to the round trip, because
                // unpack reads back through the same offset it wrote. It shows
                // up here as a byte that should not have moved.
                for i in 0..PACKED_LEN {
                    cpu.memory.set_byte(PACKED + i as u16, 0xAA);
                }
                cpu.registers.accumulator = w as u8;
                call_routine(&mut cpu, PACK_WORLD_CPU, "PACK_WORLD");
                let base = map.base(w);
                let len = map.plane_bytes(w);
                for i in 0..PACKED_LEN {
                    let inside = (base..base + len).contains(&i)
                        || (base + PLANE_RESERVE..base + PLANE_RESERVE + len).contains(&i);
                    if !inside {
                        assert_eq!(
                            cpu.memory.get_byte(PACKED + i as u16),
                            0xAA,
                            "seed {seed} W{}: pack wrote outside its slices, at +{i}",
                            w + 1,
                        );
                    }
                }
                // Scrub both halves so only the packed copy can supply them.
                for i in 0..128u16 {
                    cpu.memory.set_byte(0x7D00 + i, 0xAA);
                }
                cpu.registers.accumulator = w as u8;
                call_routine(&mut cpu, UNPACK_WORLD_CPU, "UNPACK_WORLD");
                let mask = map.mask(w);
                for (col, &m) in mask.iter().enumerate() {
                    assert_eq!(
                        cpu.memory.get_byte(0x7D00 + col as u16),
                        mario[col] & m,
                        "seed {seed} W{} column {col}: Mario half",
                        w + 1,
                    );
                    assert_eq!(
                        cpu.memory.get_byte(0x7D40 + col as u16),
                        mirror[col] & m,
                        "seed {seed} W{} column {col}: mirror half",
                        w + 1,
                    );
                }
            }
        }
    }

    /// Both hooks displace whole instructions, and the bytes they displace are
    /// what this module claims.
    #[test]
    fn hooks_displace_whole_instructions() {
        let Some(rom) = vanilla() else {
            eprintln!("SKIP: requires the ROM");
            return;
        };
        assert_eq!(
            rom.read_range(WIPE_OFFSET, WIPE_LEN),
            WIPE_VANILLA,
            "the Map_Completions wipe has moved"
        );
        assert_eq!(
            rom.read_range(RELOAD_CALL_OFFSET, RELOAD_CALL_VANILLA.len()),
            RELOAD_CALL_VANILLA,
            "the Map_Reload_with_Completions call has moved"
        );

        let mut jsr = [0xEA_u8; WIPE_LEN];
        jsr[0] = 0x20;
        jsr[1] = WIPE_REPLACEMENT_CPU as u8;
        jsr[2] = (WIPE_REPLACEMENT_CPU >> 8) as u8;
        asm::check(&WIPE_REPLACEMENT)
            .allocation(FS_WIPE_REPLACEMENT)
            .origin(WIPE_REPLACEMENT_CPU)
            .hook(&WIPE_VANILLA, 0, &jsr)
            .assert_ok();

        asm::check(&SWAP_AT_RELOAD)
            .allocation(FS_SWAP_AT_RELOAD)
            .origin(SWAP_AT_RELOAD_CPU)
            .hook(
                &RELOAD_CALL_VANILLA,
                0,
                &[0x20, SWAP_AT_RELOAD_CPU as u8, (SWAP_AT_RELOAD_CPU >> 8) as u8],
            )
            .assert_ok();
    }

    /// The hook has to *replace* the reload call, not skip it — the routine
    /// makes the call itself once the world is expanded.
    #[test]
    fn the_hook_still_reloads_the_map() {
        assert_eq!(SWAP_AT_RELOAD[14], 0x20, "the displaced JSR must be replayed");
        assert_eq!(
            u16::from_le_bytes([SWAP_AT_RELOAD[15], SWAP_AT_RELOAD[16]]),
            u16::from_le_bytes([RELOAD_CALL_VANILLA[1], RELOAD_CALL_VANILLA[2]]),
            "and it must call the same routine vanilla did"
        );
    }

    /// The packed region and the stencil scratch must not overlap each other,
    /// the counters, or `world_persist`'s arrival variables — they share two
    /// SRAM runs and nothing in the build would notice a collision.
    #[test]
    fn sram_allocations_do_not_overlap() {
        let mut used: Vec<(u16, u16, &str)> = vec![
            (PACKED, PACKED + PACKED_LEN as u16, "packed planes"),
            (MASK_SCRATCH, MASK_SCRATCH + 64, "stencil scratch"),
            (COL_IDX, COL_IDX + 1, "COL_IDX"),
            (COLS_LEFT, COLS_LEFT + 1, "COLS_LEFT"),
            (BITCNT, BITCNT + 1, "BITCNT"),
            (LIVE_WORLD, LIVE_WORLD + 1, "LIVE_WORLD"),
            (TRANSITION_FLAG, TRANSITION_FLAG + 1, "TRANSITION_FLAG"),
            (0x7AB6, 0x7ABC, "world_persist arrival vars"),
        ];
        used.sort();
        for pair in used.windows(2) {
            assert!(
                pair[0].1 <= pair[1].0,
                "{} ({:#06X}..{:#06X}) overlaps {} ({:#06X}..{:#06X})",
                pair[0].2,
                pair[0].0,
                pair[0].1,
                pair[1].2,
                pair[1].0,
                pair[1].1,
            );
        }
        // Both runs are the ones the POC proved free at runtime. Const-folded,
        // but stated here because moving either allocation out of its run is
        // otherwise silent.
        const {
            assert!(PACKED >= 0x7997 && PACKED as usize + PACKED_LEN <= 0x7A00);
            assert!(MASK_SCRATCH >= 0x7A73 && TRANSITION_FLAG <= 0x7ADF);
        }
    }
    /// **The playtest, on the emulated CPU.** Beat something in World 1, cycle
    /// all eight worlds, and it must still be beaten.
    ///
    /// This drives the two hook routines rather than the transfer routines, so
    /// it covers what the earlier tests deliberately did not: the transition
    /// flag, `LIVE_WORLD`, and the fact that `WIPE_REPLACEMENT` reads `$7D00`
    /// for the world being *left* while `SWAP_AT_RELOAD` writes it for the one
    /// being entered.
    #[test]
    fn a_beaten_level_survives_a_full_cycle() {
        let Some(rom) = vanilla() else {
            eprintln!("SKIP: requires the ROM");
            return;
        };
        let map = CompletionMap::from_rom(&rom);
        let mut cpu = cpu_with_routines(&rom);
        // The banking calls are no-ops here: PRG012 is already at $A000 for
        // the whole test, which is the arrangement they exist to produce.
        cpu.memory.set_byte(PRGROM_CHANGE_A000, 0x60);
        // And the reload is the engine's, not ours.
        cpu.memory.set_byte(MAP_RELOAD, 0x60);
        for i in 0..PACKED_LEN {
            cpu.memory.set_byte(PACKED + i as u16, 0xAA);
        }

        // --- settle on World 1 with no jump pending ---
        cpu.memory.set_byte(WORLD_NUM, 0);
        cpu.memory.set_byte(TRANSITION_FLAG, 0);
        call_routine(&mut cpu, WIPE_REPLACEMENT_CPU, "settle");
        call_routine(&mut cpu, SWAP_AT_RELOAD_CPU, "enter W1");

        // --- beat something: set a bit World 1 actually owns ---
        let (col, mask) = map
            .mask(0)
            .iter()
            .enumerate()
            .find(|&(_, &m)| m != 0)
            .map(|(c, &m)| (c, m))
            .expect("World 1 owns at least one completion bit");
        let bit = 1u8 << mask.trailing_zeros();
        cpu.memory.set_byte(0x7D00 + col as u16, bit);

        // --- the turn ends: the map re-inits at $84D7, skipping the wipe ---
        //
        // This is what the first playtest failed on. `PRG030_87A9` jumps to
        // `$84D7` on every turn end — entering a level, coming back from one,
        // losing a life — which runs everything after the wipe, this hook
        // included, with no pack in front of it. Expanding unconditionally puts
        // the slice saved on entry back over the bit just earned.
        for _ in 0..3 {
            call_routine(&mut cpu, SWAP_AT_RELOAD_CPU, "redraw");
            assert_eq!(
                cpu.memory.get_byte(0x7D00 + col as u16) & bit,
                bit,
                "a redraw of the same world must not expand a stale slice over it",
            );
        }

        // --- cycle: 1 -> 2 -> ... -> 8 -> 1 ---
        for step in 1..=8u8 {
            let dest = step % 8;
            cpu.memory.set_byte(TRANSITION_FLAG, 1); // what the jump routine does
            cpu.memory.set_byte(WORLD_NUM, dest);
            call_routine(&mut cpu, WIPE_REPLACEMENT_CPU, "leave");
            call_routine(&mut cpu, SWAP_AT_RELOAD_CPU, "enter");
            assert_eq!(cpu.memory.get_byte(LIVE_WORLD), dest, "after step {step}");
        }

        assert_eq!(
            cpu.memory.get_byte(0x7D00 + col as u16) & bit,
            bit,
            "World 1 column {col} bit {bit:#04X} was lost on the way round",
        );
    }
    /// **`PRG030_84A0` is entered far more often than a world changes**, and
    /// every one of those entries must leave `Map_Completions` exactly as it
    /// found it. Vanilla wipes it there and can afford to, because vanilla only
    /// moves forward through worlds; a maze returns to them.
    ///
    /// This is the invariant both playtest failures broke — first by zeroing
    /// the array on the non-transition arm, then by parking `LIVE_WORLD` out of
    /// range so the reload hook expanded a zeroed plane over it. Either way the
    /// level you just beat was gone before you pressed anything.
    #[test]
    fn a_non_transition_entry_never_touches_completions() {
        let Some(rom) = vanilla() else {
            eprintln!("SKIP: requires the ROM");
            return;
        };
        let mut cpu = cpu_with_routines(&rom);
        cpu.memory.set_byte(PRGROM_CHANGE_A000, 0x60);
        cpu.memory.set_byte(MAP_RELOAD, 0x60);

        // Mid-world state: progress on the map, no jump pending.
        let live: Vec<u8> = (0..128u16).map(|i| (i.wrapping_mul(37) ^ 0x5A) as u8).collect();
        for (i, &b) in live.iter().enumerate() {
            cpu.memory.set_byte(0x7D00 + i as u16, b);
        }
        cpu.memory.set_byte(WORLD_NUM, 3);
        cpu.memory.set_byte(LIVE_WORLD, 3);
        cpu.memory.set_byte(TRANSITION_FLAG, 0);

        // Entering a level, coming back, losing a life — all of them land here.
        for pass in 0..4 {
            call_routine(&mut cpu, WIPE_REPLACEMENT_CPU, "wipe replacement");
            call_routine(&mut cpu, SWAP_AT_RELOAD_CPU, "reload hook");
            for i in 0..128u16 {
                assert_eq!(
                    cpu.memory.get_byte(0x7D00 + i),
                    live[i as usize],
                    "pass {pass}: byte {i} of Map_Completions was disturbed",
                );
            }
        }
    }
}
